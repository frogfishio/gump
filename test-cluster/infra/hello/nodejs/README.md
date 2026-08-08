# k2hello

Minimal reference relying party for the constellation light-SSO flow.

This service is intentionally tiny. Its job is not to be a full application. Its job is to show the exact contract another application must implement to participate in the constellation.

## What it demonstrates

1. redirect unauthenticated users to `k2login` with a `next` URL
2. receive a one-shot handoff `nonce` from the shared login service
3. redeem that nonce server-side through `k2login`
4. persist the resulting session in a signed HTTP-only cookie
5. render a protected page and expose the session at `/session`

## Why this exists

This example is the reference implementation for “how to join the constellation”.

It shows the light-SSO model in its smallest useful form:

- shared browser sign-in happens at `k2login`
- the relying app keeps its own local session
- the relying app does not receive the login-domain refresh token
- the relying app owns its own UI after the handoff

If another project wants to participate, it should copy this pattern.

## Current flow

1. `GET /` checks for `hello_session`
2. if no local session exists, redirect to `HELLO_LOGIN_ORIGIN/?next=<this page>`
3. after login, the browser returns with `?nonce=...`
4. the server redeems that nonce with `POST ${HELLO_LOGIN_HANDOFF_BASE_URL}/handoff/redeem`
5. the server stores the returned identity envelope in a signed HTTP-only cookie
6. the browser is redirected to the same page without the nonce query parameter
7. subsequent requests use the local cookie only

## Logout flow

Hello treats logout as a shared logout across both layers of session state:

1. clear the local `hello_session` cookie
2. redirect the browser to `${HELLO_LOGIN_ORIGIN}/logout?next=${HELLO_PUBLIC_ORIGIN}/logged-out`

That second step is required.

If hello cleared only its own cookie, the browser would be redirected back to `k2login`, `k2login` would see a valid refresh cookie, and the user would appear to be silently logged back in.

The `/logged-out` route exists so the browser lands on a public page after both cookies have been cleared.

## Environment

- `HELLO_PUBLIC_ORIGIN`
- `HELLO_LOGIN_ORIGIN`
- `HELLO_LOGIN_HANDOFF_BASE_URL`
- `HELLO_SESSION_SECRET`
- `HELLO_COOKIE_NAME` optional, default `hello_session`
- `HELLO_COOKIE_SECURE` optional, default `true`
- `HELLO_LISTEN_HOST` optional, default `0.0.0.0`
- `HELLO_LISTEN_PORT` optional, default `4300`

## Routes

- `GET /health`
- `GET /ready`
- `GET /`
- `GET /session`
- `GET /logout`
- `GET /logged-out`

## Handoff contract

The hello app expects the shared login service to return the browser to the relying app as:

```text
https://hello.frogfish.io/?nonce=k2lh_...
```

It then redeems the nonce with:

```http
POST /handoff/redeem
Content-Type: application/json

{ "nonce": "k2lh_..." }
```

The response currently contains:

```json
{
  "access_token": "...",
  "token_type": "Bearer",
  "expires_in": 3600,
  "accountId": "...",
  "userId": "...",
  "roles": ["member", "admin"]
}
```

Hello then stores a signed cookie containing the local session envelope.

## Notes for other projects

- Redeem the nonce on the server, not in browser JavaScript.
- Treat the nonce as one-shot and short-lived.
- Remove the nonce from the browser URL after successful redemption.
- Keep your application session separate from the login-domain session.
- Do not try to share the refresh token across all participating apps.
- On logout, clear your app cookie and then forward the browser to `k2login /logout` so the shared login cookie is cleared too.

For a copyable implementation checklist and the full browser-to-app handoff diagram, see `docs/light-sso-constellation.md`.

## Public example

- `https://hello.frogfish.io`
- `https://login.frogfish.io`

The full architectural explanation lives in `docs/light-sso-constellation.md`.