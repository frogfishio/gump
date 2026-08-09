# Hiccup v1 Contract Corpus

This directory contains the language-neutral application-facing Hiccup HTTP
contract.

- `http.schema.json` validates bounded request and response shapes.
- `request.example.json` is a current-peer POST from Gump to an application.
- `response.example.json` is the application's current health declaration.
- `http-origin.response.example.json` is an application's `http.origin/1`
  declaration.
- `http-origin.request.example.json` is Gump's stamped capability delivery to
  Kismet, including the transitional `topic` + `data` projection.

Implementations must also generate invalid and boundary cases required by
[`docs/v1/CONFORMANCE.md`](../../../docs/v1/CONFORMANCE.md), including legacy
health, partial detection, bad token, invalid topic, forged identity/IP input,
replacement, health expiry, rotation under more than 256 peers, maximum
size/depth, and unknown version.

The internal Gump agent/keeper schema is
[`proto/gump/v1/hiccup.proto`](../../../proto/gump/v1/hiccup.proto).
