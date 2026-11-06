// The security headers every response from ihsg.midoriresearch.com carries.
//
// Two places apply them: the Worker sets them on what it answers itself
// (`/api/*`, `/healthz` and the routes wrangler.toml sends through it first),
// and `public/_headers` sets them on the static assets the assets layer
// serves without running the Worker (`/`, `/terminal`, scripts, styles, the
// 404 page). test/headers.test.mjs fails when the two lists differ.

export const SECURITY_HEADERS = {
  "x-content-type-options": "nosniff",
  "x-frame-options": "DENY",
  "referrer-policy": "strict-origin-when-cross-origin",
  "permissions-policy": "camera=(), microphone=(), geolocation=()",
  "content-security-policy":
    "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; font-src https://fonts.gstatic.com; script-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
};
