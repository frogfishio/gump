// SPDX-FileCopyrightText: 2026 Alexander R. Croft
// SPDX-License-Identifier: AGPL-3.0-or-later

/// Human release version from the repository-root `VERSION` file.
pub const VERSION: &str = env!("GUMP_VERSION");

/// Monotonic build identifier from `BUILD`, or the CI `GUMP_BUILD` override.
pub const BUILD: &str = env!("GUMP_BUILD");

pub const AUTHOR: &str = "Alexander R. Croft";
pub const LICENSE: &str = "AGPL-3.0-or-later";
pub const COMMERCIAL_LICENSE_URL: &str = "https://frogfish.io";

pub fn version_string() -> String {
    format!("{VERSION}+build-{BUILD}")
}

pub fn copyright_lines() -> [String; 2] {
    [
        format!("Copyright (C) 2026 {AUTHOR}"),
        format!("Licensed {LICENSE}; commercial licensing available at {COMMERCIAL_LICENSE_URL}"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_and_legal_notice_have_stable_cli_shape() {
        assert_eq!(version_string(), format!("{VERSION}+build-{BUILD}"));
        assert_eq!(copyright_lines().len(), 2);
        assert!(copyright_lines()[1].contains(LICENSE));
        assert!(copyright_lines()[1].contains(COMMERCIAL_LICENSE_URL));
    }
}
