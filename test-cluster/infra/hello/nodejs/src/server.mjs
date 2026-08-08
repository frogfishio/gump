import crypto from 'node:crypto';
import http from 'node:http';

const listenHost = process.env.HELLO_LISTEN_HOST || '0.0.0.0';
const listenPort = Number.parseInt(process.env.HELLO_LISTEN_PORT || '4300', 10);
const publicOrigin = requiredUrl('HELLO_PUBLIC_ORIGIN');
const loginOrigin = requiredUrl('HELLO_LOGIN_ORIGIN');
const handoffBaseUrl = requiredUrl('HELLO_LOGIN_HANDOFF_BASE_URL');
const sessionSecret = requiredEnv('HELLO_SESSION_SECRET');
const cookieName = process.env.HELLO_COOKIE_NAME || 'hello_session';
const cookieSecure = parseBoolean(process.env.HELLO_COOKIE_SECURE, true);

const server = http.createServer(async (request, response) => {
  try {
    await handleRequest(request, response);
  } catch (error) {
    console.error('[k2hello] request failed', error);
    writeJson(response, 500, { ok: false, error: 'internal_error' });
  }
});

server.listen(listenPort, listenHost, () => {
  console.log(`[k2hello] listening on http://${listenHost}:${listenPort}`);
});

async function handleRequest(request, response) {
  const requestUrl = getRequestUrl(request);

  if (request.method === 'GET' && requestUrl.pathname === '/health') {
    return writeJson(response, 200, { ok: true });
  }
  if (request.method === 'GET' && requestUrl.pathname === '/ready') {
    return writeJson(response, 200, { ok: true });
  }
  if (request.method === 'GET' && requestUrl.pathname === '/logout') {
    return redirect(response, buildSharedLogoutUrl(), {
      clearCookie: true,
    });
  }
  if (request.method === 'GET' && requestUrl.pathname === '/logged-out') {
    return writeHtml(response, 200, renderLoggedOutPage());
  }

  const cookies = parseCookies(request.headers.cookie || '');
  const cookieSession = readSessionCookie(cookies[cookieName]);
  const cleanedUrl = new URL(requestUrl.toString());
  cleanedUrl.searchParams.delete('nonce');

  const nonce = requestUrl.searchParams.get('nonce');
  if (nonce) {
    const redeemed = await redeemNonce(nonce);
    if (!redeemed) {
      return redirect(response, buildLoginUrl(cleanedUrl), { clearCookie: true });
    }

    return redirect(response, cleanedUrl.toString(), {
      session: redeemed,
    });
  }

  const session = isSessionValid(cookieSession) ? cookieSession : null;

  if (request.method === 'GET' && requestUrl.pathname === '/session') {
    if (!session) {
      return writeJson(response, 401, {
        ok: false,
        error: 'authentication_required',
        login: buildLoginUrl(cleanedUrl),
      }, sessionCookieHeaders(null));
    }

    return writeJson(response, 200, {
      ok: true,
      authenticated: true,
      session,
    });
  }

  if (request.method !== 'GET' || requestUrl.pathname !== '/') {
    return writeText(response, 404, 'Not found');
  }

  if (!session) {
    return redirect(response, buildLoginUrl(cleanedUrl), { clearCookie: !!cookieSession });
  }

  return writeHtml(response, 200, renderHelloPage(session));
}

function requiredEnv(name) {
  const value = process.env[name];
  if (!value) {
    throw new Error(`${name} is required`);
  }
  return value;
}

function requiredUrl(name) {
  const value = requiredEnv(name);
  const url = new URL(value);
  if (url.protocol !== 'http:' && url.protocol !== 'https:') {
    throw new Error(`${name} must be http(s)`);
  }
  return value.replace(/\/$/, '');
}

function parseBoolean(value, fallback) {
  if (value == null || value === '') {
    return fallback;
  }
  return ['1', 'true', 'yes', 'on'].includes(value.toLowerCase());
}

function getRequestUrl(request) {
  const host = request.headers.host || new URL(publicOrigin).host;
  const forwardedProto = splitHeaderValue(request.headers['x-forwarded-proto']);
  const protocol = forwardedProto || new URL(publicOrigin).protocol.replace(':', '');
  return new URL(request.url || '/', `${protocol}://${host}`);
}

function splitHeaderValue(value) {
  if (typeof value !== 'string') {
    return '';
  }
  return value.split(',')[0].trim();
}

function parseCookies(rawCookieHeader) {
  const cookies = {};
  for (const part of rawCookieHeader.split(';')) {
    const segment = part.trim();
    if (!segment) {
      continue;
    }
    const separatorIndex = segment.indexOf('=');
    if (separatorIndex === -1) {
      continue;
    }
    const name = segment.slice(0, separatorIndex).trim();
    const value = segment.slice(separatorIndex + 1).trim();
    cookies[name] = value;
  }
  return cookies;
}

function buildLoginUrl(nextUrl) {
  const target = new URL(`${loginOrigin}/`);
  target.searchParams.set('next', nextUrl.toString());
  return target.toString();
}

function buildSharedLogoutUrl() {
  const target = new URL(`${loginOrigin}/logout`);
  target.searchParams.set('next', `${publicOrigin}/logged-out`);
  return target.toString();
}

async function redeemNonce(nonce) {
  const payload = await loginJson('/handoff/redeem', {
    method: 'POST',
    body: { nonce },
  });

  if (!payload || typeof payload.access_token !== 'string') {
    return null;
  }

  const claims = decodeJwtPayload(payload.access_token);

  return {
    accessToken: payload.access_token,
    tokenType: payload.token_type || 'Bearer',
    expiresAtMs: computeExpiryMs(payload.access_token, payload.expires_in),
    accountId: payload.accountId || claims.account || '',
    userId: payload.userId || claims.sub || '',
    roles: Array.isArray(payload.roles) ? payload.roles : [],
    permissions: Array.isArray(payload.permissions) ? payload.permissions : [],
    claims,
  };
}

async function loginJson(path, options) {
  const headers = {
    Accept: 'application/json',
  };

  if (options.authorization) {
    headers.Authorization = options.authorization;
  }

  let body;
  if (options.body !== undefined) {
    headers['Content-Type'] = 'application/json';
    body = JSON.stringify(options.body);
  }

  const response = await fetch(`${handoffBaseUrl}${path}`, {
    method: options.method,
    headers,
    body,
    signal: AbortSignal.timeout(8000),
  });

  if (!response.ok) {
    return null;
  }

  return response.json();
}

function decodeJwtPayload(token) {
  const parts = token.split('.');
  if (parts.length !== 3) {
    return {};
  }

  try {
    return JSON.parse(Buffer.from(base64UrlToBase64(parts[1]), 'base64').toString('utf8'));
  } catch {
    return {};
  }
}

function computeExpiryMs(token, expiresInSeconds) {
  const claims = decodeJwtPayload(token);
  if (typeof claims.exp === 'number') {
    return claims.exp * 1000;
  }
  if (typeof expiresInSeconds === 'number') {
    return Date.now() + Math.max(0, expiresInSeconds - 30) * 1000;
  }
  return Date.now() + 15 * 60 * 1000;
}

function isSessionValid(session) {
  return !!session && typeof session.expiresAtMs === 'number' && session.expiresAtMs > Date.now();
}

function readSessionCookie(rawValue) {
  if (!rawValue) {
    return null;
  }

  const separatorIndex = rawValue.lastIndexOf('.');
  if (separatorIndex === -1) {
    return null;
  }

  const payloadPart = rawValue.slice(0, separatorIndex);
  const signaturePart = rawValue.slice(separatorIndex + 1);
  const expected = sign(payloadPart);
  if (!timingSafeEqual(signaturePart, expected)) {
    return null;
  }

  try {
    return JSON.parse(Buffer.from(base64UrlToBase64(payloadPart), 'base64').toString('utf8'));
  } catch {
    return null;
  }
}

function writeSessionCookie(session) {
  if (!session) {
    return serializeCookie(cookieName, '', {
      path: '/',
      httpOnly: true,
      sameSite: 'Lax',
      secure: cookieSecure,
      maxAge: 0,
    });
  }

  const payloadPart = Buffer.from(JSON.stringify(session)).toString('base64url');
  const signed = `${payloadPart}.${sign(payloadPart)}`;
  const maxAgeSeconds = Math.max(0, Math.floor((session.expiresAtMs - Date.now()) / 1000));
  return serializeCookie(cookieName, signed, {
    path: '/',
    httpOnly: true,
    sameSite: 'Lax',
    secure: cookieSecure,
    maxAge: maxAgeSeconds,
  });
}

function sessionCookieHeaders(session) {
  return {
    'Set-Cookie': writeSessionCookie(session),
  };
}

function sign(payloadPart) {
  return crypto.createHmac('sha256', sessionSecret).update(payloadPart).digest('base64url');
}

function timingSafeEqual(left, right) {
  const leftBuffer = Buffer.from(left);
  const rightBuffer = Buffer.from(right);
  if (leftBuffer.length !== rightBuffer.length) {
    return false;
  }
  return crypto.timingSafeEqual(leftBuffer, rightBuffer);
}

function base64UrlToBase64(value) {
  let output = value.replace(/-/g, '+').replace(/_/g, '/');
  while (output.length % 4 !== 0) {
    output += '=';
  }
  return output;
}

function serializeCookie(name, value, options) {
  const parts = [`${name}=${value}`];
  if (options.maxAge !== undefined) {
    parts.push(`Max-Age=${options.maxAge}`);
  }
  if (options.path) {
    parts.push(`Path=${options.path}`);
  }
  if (options.httpOnly) {
    parts.push('HttpOnly');
  }
  if (options.sameSite) {
    parts.push(`SameSite=${options.sameSite}`);
  }
  if (options.secure) {
    parts.push('Secure');
  }
  return parts.join('; ');
}

function redirect(response, location, options = {}) {
  const headers = {
    Location: location,
    ...options.headers,
  };
  if (options.clearCookie) {
    headers['Set-Cookie'] = writeSessionCookie(null);
  }
  if (options.session) {
    headers['Set-Cookie'] = writeSessionCookie(options.session);
  }
  response.writeHead(302, headers);
  response.end();
}

function writeJson(response, statusCode, payload, extraHeaders = {}) {
  const body = JSON.stringify(payload, null, 2);
  response.writeHead(statusCode, {
    'Content-Type': 'application/json; charset=utf-8',
    'Cache-Control': 'no-store',
    'Content-Length': Buffer.byteLength(body),
    ...extraHeaders,
  });
  response.end(body);
}

function writeText(response, statusCode, body) {
  response.writeHead(statusCode, {
    'Content-Type': 'text/plain; charset=utf-8',
    'Cache-Control': 'no-store',
    'Content-Length': Buffer.byteLength(body),
  });
  response.end(body);
}

function writeHtml(response, statusCode, body) {
  response.writeHead(statusCode, {
    'Content-Type': 'text/html; charset=utf-8',
    'Cache-Control': 'no-store',
    'Content-Length': Buffer.byteLength(body),
  });
  response.end(body);
}

function formatTimestamp(value) {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return '-';
  }
  return new Date(value).toISOString();
}

function renderValueList(values) {
  if (!Array.isArray(values) || values.length === 0) {
    return '<span class="empty">-</span>';
  }
  return values.map((value) => `<li>${escapeHtml(String(value))}</li>`).join('');
}

function renderRows(rows) {
  return rows.map(([label, value]) => `
                  <tr>
                    <th scope="row">${escapeHtml(label)}</th>
                    <td>${value}</td>
                  </tr>`).join('');
}

function renderHelloPage(session) {
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>k2hello</title>
    <style>
      :root {
        color-scheme: light;
        --bg: #f4f6f8;
        --panel: #ffffff;
        --panel-alt: #f8fafc;
        --border: #d7dee7;
        --border-strong: #b8c4d3;
        --text: #16202b;
        --muted: #526173;
        --heading: #0f1720;
        --accent: #0b5cab;
        --accent-soft: #e8f1fb;
        --code-bg: #0f1720;
        --code-text: #e6edf5;
        --shadow: 0 1px 2px rgba(15, 23, 32, 0.06);
      }
      * { box-sizing: border-box; }
      html, body { min-height: 100%; }
      body {
        margin: 0;
        font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
        color: var(--text);
        background: var(--bg);
        line-height: 1.45;
      }
      a {
        color: var(--accent);
      }
      code,
      pre,
      .mono {
        font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', monospace;
      }
      .shell {
        max-width: 1480px;
        margin: 0 auto;
        padding: 24px;
      }
      .masthead {
        display: flex;
        justify-content: space-between;
        align-items: flex-start;
        gap: 20px;
        padding: 20px 24px;
        border: 1px solid var(--border);
        background: var(--panel);
        box-shadow: var(--shadow);
      }
      .eyebrow {
        margin: 0 0 6px;
        font-size: 12px;
        letter-spacing: 0.08em;
        text-transform: uppercase;
        color: var(--muted);
        font-weight: 700;
      }
      h1, h2, h3 {
        margin: 0;
        color: var(--heading);
      }
      h1 {
        font-size: 32px;
        line-height: 1.15;
      }
      .lede {
        max-width: 90ch;
        margin: 10px 0 0;
        color: var(--muted);
        font-size: 15px;
      }
      .status-bar {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 12px;
        margin-top: 12px;
        color: var(--muted);
        font-size: 14px;
      }
      .actions {
        display: flex;
        gap: 12px;
        flex-wrap: wrap;
      }
      .button {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        min-height: 42px;
        padding: 0 16px;
        border: 1px solid var(--border-strong);
        text-decoration: none;
        font-weight: 600;
        background: var(--panel);
        color: var(--text);
      }
      .button.primary {
        background: var(--accent);
        border-color: var(--accent);
        color: #ffffff;
      }
      .button.secondary {
        background: var(--accent-soft);
        color: var(--accent);
        border-color: #c9d9ec;
      }
      .layout {
        display: grid;
        gap: 18px;
        margin-top: 18px;
      }
      .summary-grid {
        display: grid;
        grid-template-columns: 1.1fr 0.9fr;
        gap: 18px;
      }
      .token-grid {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: 18px;
      }
      .panel {
        border: 1px solid var(--border);
        background: var(--panel);
        box-shadow: var(--shadow);
      }
      .panel-header {
        padding: 18px 20px 0;
      }
      .panel-header h2,
      .panel-header h3 {
        font-size: 20px;
      }
      .panel-body {
        padding: 18px 20px 20px;
      }
      .kv-table,
      .flow-table {
        width: 100%;
        border-collapse: collapse;
      }
      .kv-table th,
      .kv-table td,
      .flow-table th,
      .flow-table td {
        padding: 12px 10px;
        border-top: 1px solid var(--border);
        text-align: left;
        vertical-align: top;
      }
      .kv-table tbody tr:first-child th,
      .kv-table tbody tr:first-child td,
      .flow-table thead th {
        border-top: 0;
      }
      .kv-table th,
      .flow-table th {
        width: 190px;
        font-size: 13px;
        font-weight: 700;
        color: var(--muted);
      }
      .flow-table thead th {
        background: var(--panel-alt);
        color: var(--heading);
      }
      .list-reset {
        list-style: none;
        padding: 0;
        margin: 0;
        display: flex;
        flex-wrap: wrap;
        gap: 8px;
      }
      .list-reset li {
        padding: 5px 9px;
        border: 1px solid var(--border);
        background: var(--panel-alt);
      }
      .empty {
        color: var(--muted);
      }
      .caption,
      .note {
        margin: 0;
        color: var(--muted);
      }
      pre {
        margin: 0;
        padding: 16px;
        background: var(--code-bg);
        color: var(--code-text);
        font-size: 13px;
        line-height: 1.5;
        overflow-x: auto;
        min-height: 280px;
        white-space: pre-wrap;
        word-break: break-word;
      }
      @media (max-width: 1080px) {
        .summary-grid,
        .token-grid {
          grid-template-columns: 1fr;
        }
      }
      @media (max-width: 720px) {
        .shell {
          padding: 14px;
        }
        .masthead {
          padding: 18px;
          flex-direction: column;
        }
        h1 {
          font-size: 26px;
        }
        .kv-table th,
        .kv-table td,
        .flow-table th,
        .flow-table td {
          display: block;
          width: auto;
          padding: 8px 0;
        }
        .flow-table thead {
          display: none;
        }
        .flow-table tbody tr {
          display: block;
          padding: 12px 0;
          border-top: 1px solid var(--border);
        }
      }
    </style>
  </head>
  <body>
    <main class="shell">
      <section class="masthead">
        <div>
          <p class="eyebrow">Reference relying party</p>
          <h1>k2hello session console</h1>
          <p class="lede">This is the smallest protected app in the constellation. It proves the server-side handoff contract: redirect to login, redeem the one-shot nonce on the server, persist a local signed cookie, and render the authenticated session state.</p>
          <div class="status-bar">
            <span>Public origin: <span class="mono">${escapeHtml(publicOrigin)}</span></span>
            <span>Login origin: <span class="mono">${escapeHtml(loginOrigin)}</span></span>
          </div>
        </div>
        <div class="actions">
          <a class="button primary" href="/session">View Session JSON</a>
          <a class="button secondary" href="/logout">Log out</a>
        </div>
      </section>

      <section class="layout">
        <div class="summary-grid">
          <article class="panel">
            <div class="panel-header">
              <p class="eyebrow">Session envelope</p>
              <h2>Identity and expiry</h2>
            </div>
            <div class="panel-body">
              <table class="kv-table">
                <tbody>${renderRows([
                  ['Account ID', `<span class="mono">${escapeHtml(session.accountId || '-')}</span>`],
                  ['User ID', `<span class="mono">${escapeHtml(session.userId || '-')}</span>`],
                  ['Token type', `<span class="mono">${escapeHtml(session.tokenType || '-')}</span>`],
                  ['Expires at', `<span class="mono">${escapeHtml(formatTimestamp(session.expiresAtMs))}</span>`],
                  ['Roles', `<ul class="list-reset">${renderValueList(session.roles)}</ul>`],
                  ['Permissions', `<ul class="list-reset">${renderValueList(session.permissions)}</ul>`],
                ])}</tbody>
              </table>
            </div>
          </article>

          <article class="panel">
            <div class="panel-header">
              <p class="eyebrow">Contract</p>
              <h2>Browser to app handoff</h2>
            </div>
            <div class="panel-body">
              <table class="flow-table">
                <thead>
                  <tr>
                    <th scope="col">Step</th>
                    <th scope="col">What happens</th>
                  </tr>
                </thead>
                <tbody>
                  <tr>
                    <th scope="row">1. Redirect</th>
                    <td>Unauthenticated requests are redirected to <span class="mono">${escapeHtml(loginOrigin)}/?next=...</span>.</td>
                  </tr>
                  <tr>
                    <th scope="row">2. Login</th>
                    <td>k2login authenticates the browser and returns to this app with <span class="mono">?nonce=...</span>.</td>
                  </tr>
                  <tr>
                    <th scope="row">3. Redeem</th>
                    <td>This server calls <span class="mono">${escapeHtml(handoffBaseUrl)}/handoff/redeem</span> and exchanges the nonce for a JWT payload.</td>
                  </tr>
                  <tr>
                    <th scope="row">4. Persist</th>
                    <td>The resulting session envelope is stored in a signed, HTTP-only local cookie and the page reloads without the nonce parameter.</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </article>
        </div>

        <div class="token-grid">
          <article class="panel">
            <div class="panel-header">
              <p class="eyebrow">Access token</p>
              <h2>JWT returned by k2login</h2>
            </div>
            <div class="panel-body">
              <p class="caption">This is the raw access token stored inside the local hello session envelope.</p>
              <pre><code>${escapeHtml(session.accessToken)}</code></pre>
            </div>
          </article>

          <article class="panel">
            <div class="panel-header">
              <p class="eyebrow">Claims</p>
              <h2>Decoded token payload</h2>
            </div>
            <div class="panel-body">
              <p class="caption">Claims are decoded locally for inspection only; the server still treats the signed cookie as the active app session container.</p>
              <pre><code>${escapeHtml(JSON.stringify(session.claims, null, 2))}</code></pre>
            </div>
          </article>
        </div>

        <article class="panel">
          <div class="panel-header">
            <p class="eyebrow">Operational notes</p>
            <h2>Why this page exists</h2>
          </div>
          <div class="panel-body">
            <p class="note">k2hello is not intended to be a marketing surface. It is a reference integration for developers adopting the constellation login handoff. The page is intentionally explicit about what was redeemed, what was stored, and where the trust boundary sits.</p>
          </div>
        </article>
      </section>
    </main>
  </body>
</html>`;
}

function renderLoggedOutPage() {
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>k2hello logged out</title>
    <style>
      :root {
        color-scheme: light;
        --bg: #f4f6f8;
        --panel: #ffffff;
        --border: #d7dee7;
        --text: #16202b;
        --muted: #526173;
        --accent: #0b5cab;
        --accent-soft: #e8f1fb;
        --shadow: 0 1px 2px rgba(15, 23, 32, 0.06);
      }
      * { box-sizing: border-box; }
      body {
        margin: 0;
        background: var(--bg);
        color: var(--text);
        font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
      }
      .shell {
        max-width: 1080px;
        margin: 0 auto;
        padding: 24px;
      }
      .panel {
        border: 1px solid var(--border);
        background: var(--panel);
        box-shadow: var(--shadow);
        padding: 24px;
      }
      .eyebrow {
        margin: 0 0 6px;
        font-size: 12px;
        letter-spacing: 0.08em;
        text-transform: uppercase;
        color: var(--muted);
        font-weight: 700;
      }
      h1 {
        margin: 0;
        font-size: 30px;
      }
      p {
        margin: 12px 0 0;
        max-width: 75ch;
        color: var(--muted);
        line-height: 1.5;
      }
      .actions {
        margin-top: 20px;
        display: flex;
        gap: 12px;
        flex-wrap: wrap;
      }
      .button {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        min-height: 42px;
        padding: 0 16px;
        border: 1px solid var(--border);
        text-decoration: none;
        font-weight: 600;
      }
      .button.primary {
        background: var(--accent);
        border-color: var(--accent);
        color: #ffffff;
      }
      .button.secondary {
        background: var(--accent-soft);
        color: var(--accent);
      }
    </style>
  </head>
  <body>
    <main class="shell">
      <section class="panel">
        <p class="eyebrow">Reference relying party</p>
        <h1>Shared logout completed</h1>
        <p>Your local hello session cookie and the shared login refresh cookie have been cleared. Use the login button to begin a fresh end-to-end handoff, or return directly to the login origin.</p>
        <div class="actions">
          <a class="button primary" href="${escapeHtml(buildLoginUrl(new URL(`${publicOrigin}/`)))}">Sign in again</a>
          <a class="button secondary" href="${escapeHtml(loginOrigin)}/">Go to login</a>
        </div>
      </section>
    </main>
  </body>
</html>`;
}

function escapeHtml(value) {
  return String(value)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}
