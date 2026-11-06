//! Offline ephemeris for `market astro`: apparent positions of the Sun, Moon
//! and planets, lunar phases, stations and sign ingresses, Mercury's
//! retrograde periods, the Javanese weton, the Chinese shio and IHSG's own
//! chart. Pure computation, no network, no new crates; checked against JPL
//! Horizons, USNO and published calendars in `tests/fixtures/astro/`.
//!
//! `web/src/astro.js` is a line-for-line port: same tables, same steps, same
//! rounding, so the CLI, the MCP server and the web terminal agree.

pub mod ephem;
pub mod events;
pub mod moon;
pub mod time;
