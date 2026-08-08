//! POSIX ustar writer/parser with Gump normalization rules.

use std::io::Write;

use super::error::{ArchiveError, ArchiveErrorKind};
use super::pack::{ArchiveEntry, EntryKind};
use super::path::validate_archive_path;

const BLOCK: usize = 512;
const USTAR_MAGIC: &[u8; 6] = b"ustar\0";
const USTAR_VERSION: &[u8; 2] = b"00";

/// Serialize entries to a deterministic ustar byte stream (sorted by path).
pub fn write_ustar(entries: &[ArchiveEntry]) -> Result<Vec<u8>, ArchiveError> {
    let mut out = Vec::new();
    write_ustar_to(entries, &mut out)?;
    Ok(out)
}

/// Stream a deterministic ustar archive into `out` (sorted by path).
///
/// Does not buffer the full archive: headers and file payloads are written
/// entry-by-entry (STL-14 / CONFORMANCE streaming extract contract).
pub fn write_ustar_to<W: Write>(entries: &[ArchiveEntry], out: &mut W) -> Result<(), ArchiveError> {
    let order = sorted_entry_order(entries)?;
    for &i in &order {
        write_entry(out, &entries[i])?;
    }
    out.write_all(&[0u8; BLOCK])
        .map_err(|e| ArchiveError::new(ArchiveErrorKind::Io, e.to_string()))?;
    out.write_all(&[0u8; BLOCK])
        .map_err(|e| ArchiveError::new(ArchiveErrorKind::Io, e.to_string()))?;
    Ok(())
}

/// Encoded ustar byte length for `entries` in lexical path order (incl. end blocks).
pub fn ustar_encoded_len(entries: &[ArchiveEntry]) -> Result<u64, ArchiveError> {
    let order = sorted_entry_order(entries)?;
    let mut n = 0u64;
    for &i in &order {
        n = n.saturating_add(BLOCK as u64);
        if entries[i].kind == EntryKind::File {
            let len = entries[i].data.len() as u64;
            n = n.saturating_add(len);
            n = n.saturating_add((BLOCK as u64 - (len % BLOCK as u64)) % BLOCK as u64);
        }
    }
    Ok(n.saturating_add(2 * BLOCK as u64))
}

fn sorted_entry_order(entries: &[ArchiveEntry]) -> Result<Vec<usize>, ArchiveError> {
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by(|&a, &b| entries[a].path.as_bytes().cmp(entries[b].path.as_bytes()));
    for w in order.windows(2) {
        if entries[w[0]].path == entries[w[1]].path {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Path,
                format!("duplicate archive path {}", entries[w[0]].path),
            ));
        }
    }
    Ok(order)
}

/// Parse a Gump-normalized ustar stream into entries (order preserved = lexical).
pub fn parse_ustar(
    bytes: &[u8],
    max_files: usize,
    max_bytes: u64,
) -> Result<Vec<ArchiveEntry>, ArchiveError> {
    if bytes.len() % BLOCK != 0 {
        return Err(ArchiveError::new(
            ArchiveErrorKind::Format,
            "ustar length is not a multiple of 512",
        ));
    }
    let mut entries = Vec::new();
    let mut expanded: u64 = 0;
    let mut i = 0;
    while i + BLOCK <= bytes.len() {
        let header = &bytes[i..i + BLOCK];
        i += BLOCK;
        if header.iter().all(|&b| b == 0) {
            // End marker: require a second zero block if present; stop either way.
            if i + BLOCK <= bytes.len() && bytes[i..i + BLOCK].iter().all(|&b| b == 0) {
                i += BLOCK;
            }
            if i != bytes.len() {
                return Err(ArchiveError::new(
                    ArchiveErrorKind::Format,
                    "trailing data after ustar end markers",
                ));
            }
            break;
        }

        let (path, kind, mode_exec, size) = parse_header(header)?;
        if entries.len() >= max_files {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Limit,
                "file count ceiling exceeded",
            ));
        }
        expanded = expanded.saturating_add(size);
        if expanded > max_bytes {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Limit,
                "expanded size ceiling exceeded",
            ));
        }

        let data_blocks = (size as usize).div_ceil(BLOCK);
        let need = data_blocks.saturating_mul(BLOCK);
        if i + need > bytes.len() {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Format,
                "truncated file data",
            ));
        }
        let mut data = bytes[i..i + size as usize].to_vec();
        i += need;
        if kind == EntryKind::Directory {
            if !data.is_empty() {
                return Err(ArchiveError::new(
                    ArchiveErrorKind::Format,
                    "directory entry has payload",
                ));
            }
            data.clear();
        }
        entries.push(ArchiveEntry {
            path,
            kind,
            executable: mode_exec,
            data,
        });
    }
    Ok(entries)
}

fn write_entry<W: Write>(out: &mut W, entry: &ArchiveEntry) -> Result<(), ArchiveError> {
    let path = validate_archive_path(
        entry.path.trim_end_matches('/'),
        entry.kind == EntryKind::Directory,
    )?;
    if path != entry.path {
        return Err(ArchiveError::new(
            ArchiveErrorKind::Path,
            format!("path not normalized: got {:?}, want {:?}", entry.path, path),
        ));
    }

    let (prefix, name) = split_ustar_path(&path)?;
    let mut header = [0u8; BLOCK];
    put_bytes(&mut header[0..100], name.as_bytes());
    let mode = match entry.kind {
        EntryKind::Directory => 0o755u32,
        EntryKind::File if entry.executable => 0o755,
        EntryKind::File => 0o644,
    };
    put_octal(&mut header[100..108], mode as u64, 7);
    put_octal(&mut header[108..116], 0, 7); // uid
    put_octal(&mut header[116..124], 0, 7); // gid
    let size = match entry.kind {
        EntryKind::Directory => 0u64,
        EntryKind::File => entry.data.len() as u64,
    };
    put_octal(&mut header[124..136], size, 11);
    put_octal(&mut header[136..148], 0, 11); // mtime
    // checksum field filled with spaces for calculation
    header[148..156].fill(b' ');
    header[156] = match entry.kind {
        EntryKind::File => b'0',
        EntryKind::Directory => b'5',
    };
    header[257..263].copy_from_slice(USTAR_MAGIC);
    header[263..265].copy_from_slice(USTAR_VERSION);
    // uname/gname empty, devices zero
    put_bytes(&mut header[345..500], prefix.as_bytes());

    let sum: u32 = header.iter().map(|&b| b as u32).sum();
    put_octal(&mut header[148..156], sum as u64, 6);
    header[154] = 0;
    header[155] = b' ';

    out.write_all(&header)
        .map_err(|e| ArchiveError::new(ArchiveErrorKind::Io, e.to_string()))?;
    if entry.kind == EntryKind::File {
        out.write_all(&entry.data)
            .map_err(|e| ArchiveError::new(ArchiveErrorKind::Io, e.to_string()))?;
        let pad = (BLOCK - (entry.data.len() % BLOCK)) % BLOCK;
        if pad > 0 {
            out.write_all(&[0u8; BLOCK][..pad])
                .map_err(|e| ArchiveError::new(ArchiveErrorKind::Io, e.to_string()))?;
        }
    }
    Ok(())
}

pub(crate) fn parse_header(header: &[u8]) -> Result<(String, EntryKind, bool, u64), ArchiveError> {
    if &header[257..263] != USTAR_MAGIC {
        return Err(ArchiveError::new(
            ArchiveErrorKind::Format,
            "non-ustar or extended tar header rejected",
        ));
    }
    if &header[263..265] != USTAR_VERSION {
        return Err(ArchiveError::new(
            ArchiveErrorKind::Format,
            "ustar version must be \"00\"",
        ));
    }
    // Reject PAX/GNU typeflags and link/device types.
    let typeflag = header[156];
    let kind = match typeflag {
        b'0' | b'\0' => EntryKind::File,
        b'5' => EntryKind::Directory,
        _ => {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Format,
                format!("unsupported tar typeflag {}", typeflag as char),
            ));
        }
    };

    // uid/gid/mtime/names must be zero/empty for Gump archives we accept as normalized.
    if parse_octal(&header[108..116])? != 0
        || parse_octal(&header[116..124])? != 0
        || parse_octal(&header[136..148])? != 0
    {
        return Err(ArchiveError::new(
            ArchiveErrorKind::Format,
            "non-zero uid/gid/mtime rejected",
        ));
    }
    if header[265..297].iter().any(|&b| b != 0) || header[297..329].iter().any(|&b| b != 0) {
        return Err(ArchiveError::new(
            ArchiveErrorKind::Format,
            "owner/group names must be empty",
        ));
    }

    let name = cstr_field(&header[0..100])?;
    let prefix = cstr_field(&header[345..500])?;
    let raw = if prefix.is_empty() {
        name
    } else {
        format!("{prefix}/{name}")
    };
    let path = validate_archive_path(raw.trim_end_matches('/'), kind == EntryKind::Directory)?;

    let mode = parse_octal(&header[100..108])?;
    if kind == EntryKind::Directory && mode != 0o755 {
        return Err(ArchiveError::new(
            ArchiveErrorKind::Format,
            "directory mode must be 0755",
        ));
    }
    let executable = match kind {
        EntryKind::Directory => false,
        EntryKind::File => match mode {
            0o644 => false,
            0o755 => true,
            _ => {
                return Err(ArchiveError::new(
                    ArchiveErrorKind::Format,
                    format!("unsupported mode {mode:o}"),
                ));
            }
        },
    };

    let size = parse_octal(&header[124..136])?;
    // Verify checksum.
    let mut tmp = header.to_vec();
    tmp[148..156].fill(b' ');
    let sum: u32 = tmp.iter().map(|&b| b as u32).sum();
    let claimed = parse_octal(&header[148..156])?;
    if claimed != sum as u64 {
        return Err(ArchiveError::new(
            ArchiveErrorKind::Format,
            "ustar checksum mismatch",
        ));
    }
    Ok((path, kind, executable, size))
}

fn split_ustar_path(path: &str) -> Result<(String, String), ArchiveError> {
    let bytes = path.as_bytes();
    if bytes.len() <= 100 {
        return Ok((String::new(), path.to_string()));
    }
    // Prefer split on '/' so name fits in 100 and prefix in 155.
    if bytes.len() > 100 + 1 + 155 {
        return Err(ArchiveError::new(
            ArchiveErrorKind::Limit,
            "path exceeds ustar name+prefix capacity",
        ));
    }
    for split in (1..bytes.len().saturating_sub(1)).rev() {
        if bytes[split] != b'/' {
            continue;
        }
        let prefix = &path[..split];
        let name = &path[split + 1..];
        if prefix.len() <= 155 && name.len() <= 100 && !name.is_empty() {
            return Ok((prefix.to_string(), name.to_string()));
        }
    }
    Err(ArchiveError::new(
        ArchiveErrorKind::Limit,
        "path cannot be split into ustar prefix/name",
    ))
}

fn put_bytes(dst: &mut [u8], src: &[u8]) {
    let n = src.len().min(dst.len());
    dst[..n].copy_from_slice(&src[..n]);
}

fn put_octal(dst: &mut [u8], value: u64, digits: usize) {
    // NUL-terminated, zero-padded octal (common POSIX ustar layout).
    let s = format!("{:0width$o}", value, width = digits);
    let bytes = s.as_bytes();
    let n = bytes.len().min(dst.len().saturating_sub(1));
    dst[..n].copy_from_slice(&bytes[..n]);
    if n < dst.len() {
        dst[n] = 0;
    }
}

fn parse_octal(field: &[u8]) -> Result<u64, ArchiveError> {
    let end = field
        .iter()
        .position(|&b| b == 0 || b == b' ')
        .unwrap_or(field.len());
    let s = std::str::from_utf8(&field[..end])
        .map_err(|_| ArchiveError::new(ArchiveErrorKind::Format, "octal field not utf8"))?;
    if s.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(s.trim(), 8)
        .map_err(|_| ArchiveError::new(ArchiveErrorKind::Format, format!("invalid octal {s:?}")))
}

fn cstr_field(field: &[u8]) -> Result<String, ArchiveError> {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    let s = std::str::from_utf8(&field[..end])
        .map_err(|_| ArchiveError::new(ArchiveErrorKind::Format, "path field not utf8"))?;
    Ok(s.to_string())
}
