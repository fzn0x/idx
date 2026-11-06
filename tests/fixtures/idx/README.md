# idx.co.id fixtures

Raw responses of the idx.co.id JSON endpoints behind its pages (see
`src/providers/idx/mod.rs` for the endpoints and the dates they were
recorded), and the two kinds of Cloudflare refusal idx.co.id answers some
networks with.

| file | provenance | used for |
| --- | --- | --- |
| cloudflare_blocked.html | Real Cloudflare block page ("Attention Required! \| Cloudflare", "Sorry, you have been blocked"), HTTP 403 with `server: cloudflare` and no `cf-mitigated` header, recorded in October 2026. Redacted on 2026-11-06: the visitor IP is the documentation address `203.0.113.0`, the Ray ID and the per-visit `u` token in `window.__CF$cv$params` are zeros and its signed `ut` token is `REDACTED`; nothing else changed | `blocked` errors (block page), `http::cloudflare` |
| cloudflare_challenge_synthetic.html | Synthetic, labelled inside: the markers of Cloudflare's managed challenge as GitHub-hosted runners receive it ("Just a moment...", `window._cf_chl_opt`, `/cdn-cgi/challenge-platform/`). Real challenges come as HTTP 403 with the response header `cf-mitigated: challenge`; tests serve it that way | `blocked` errors (challenge), `http::cloudflare` |
