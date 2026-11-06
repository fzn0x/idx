// IHSG astrology for the Worker: a line-for-line port of src/astro/ and
// src/analysis/{astro,stats}.rs (same tables, same steps, same rounding), so
// /api/astro returns the CLI's idx.astro.v1 payload. Pure functions, no I/O.
//
// Sources: J. Meeus, "Astronomical Algorithms" 2nd ed. (ch. 21.5, 22, 23, 47, 48);
// E. M. Standish, JPL "Keplerian Elements for Approximate Positions of the Major
// Planets", Table 1 (Pluto from JPL's archived p_elem_t1.txt); P. Schlyter, "How to
// compute planetary positions" (Jupiter and Saturn); Espenak and Meeus (Delta T);
// Numerical Recipes (gammln, betacf, betai). Checked against JPL Horizons and USNO
// recordings in tests/fixtures/astro/ (web/test/astro.test.mjs).

const D2R = Math.PI / 180;
const R2D = 180 / Math.PI;
const DAY = 86400e3;
const HOUR = 3600e3;
const WIB = 7 * HOUR;
export const norm360 = (x) => ((x % 360) + 360) % 360;
const sind = (x) => Math.sin(x * D2R);
const cosd = (x) => Math.cos(x * D2R);
/** Rounds like time::round_to in Rust: Math.round, no negative zero. */
const round = (x, places) => { const k = 10 ** places; const r = Math.round(x * k) / k; return r === 0 ? 0 : r; };

// ---- time --------------------------------------------------------------------

const jdFromMs = (ms) => ms / DAY + 2440587.5;
export function deltaT(y) {
  let t;
  if (y < 1800) return 13.72;
  if (y < 1860) { t = y - 1800; return 13.72 - 0.332447 * t + 0.0068612 * t ** 2 + 0.0041116 * t ** 3 - 0.00037436 * t ** 4 + 0.0000121272 * t ** 5 - 0.0000001699 * t ** 6 + 0.000000000875 * t ** 7; }
  if (y < 1900) { t = y - 1860; return 7.62 + 0.5737 * t - 0.251754 * t ** 2 + 0.01680668 * t ** 3 - 0.0004473624 * t ** 4 + t ** 5 / 233174; }
  if (y < 1920) { t = y - 1900; return -2.79 + 1.494119 * t - 0.0598939 * t ** 2 + 0.0061966 * t ** 3 - 0.000197 * t ** 4; }
  if (y < 1941) { t = y - 1920; return 21.20 + 0.84493 * t - 0.076100 * t ** 2 + 0.0020936 * t ** 3; }
  if (y < 1961) { t = y - 1950; return 29.07 + 0.407 * t - t ** 2 / 233 + t ** 3 / 2547; }
  if (y < 1986) { t = y - 1975; return 45.45 + 1.067 * t - t ** 2 / 260 - t ** 3 / 718; }
  if (y < 2005) { t = y - 2000; return 63.86 + 0.3345 * t - 0.060374 * t ** 2 + 0.0017275 * t ** 3 + 0.000651814 * t ** 4 + 0.00002373599 * t ** 5; }
  if (y < 2050) { t = y - 2000; return 62.92 + 0.32217 * t + 0.005589 * t ** 2; }
  return -20 + 32 * ((y - 1820) / 100) ** 2 - 0.5628 * (2150 - y);
}
const jdeFromMs = (ms) => { const jd = jdFromMs(ms); return jd + deltaT(2000 + (jd - 2451544.5) / 365.2425) / 86400; };
const centuries = (jde) => (jde - 2451545.0) / 36525;
const utcMs = (iso, hour) => Date.parse(`${iso}T${String(hour).padStart(2, "0")}:00:00Z`);
/** 16:00 WIB on the date, the IDX close. */
export const closeMs = (iso) => utcMs(iso, 9);
const noonMs = (iso) => utcMs(iso, 5);
const dayStartMs = (iso) => utcMs(iso, 0) - WIB;
/** An instant rounded to the minute as a WIB RFC 3339 string. */
export const wibIso = (ms) => new Date(Math.round(ms / 60e3) * 60e3 + WIB).toISOString().slice(0, 16) + ":00+07:00";
const isoOfDay = (day) => new Date(day * DAY).toISOString().slice(0, 10);

// ---- nutation, precession ------------------------------------------------------

function nutationLon(T) {
  const om = 125.04452 - 1934.136261 * T, L = 280.4665 + 36000.7698 * T, Lp = 218.3165 + 481267.8813 * T;
  return (-17.20 * sind(om) - 1.32 * sind(2 * L) - 0.23 * sind(2 * Lp) + 0.21 * sind(2 * om)) / 3600;
}
function precessFromJ2000(lon, lat, t) {
  const eta = (47.0029 * t - 0.03302 * t * t + 0.000060 * t ** 3) / 3600;
  const Pi = 174.876384 - (869.8089 * t + 0.03536 * t * t) / 3600;
  const p = (5029.0966 * t + 1.11113 * t * t - 0.0000006 * t ** 3) / 3600;
  const A = cosd(eta) * cosd(lat) * sind(Pi - lon) - sind(eta) * sind(lat);
  const B = cosd(lat) * cosd(Pi - lon);
  const C = cosd(eta) * sind(lat) + sind(eta) * cosd(lat) * sind(Pi - lon);
  return [norm360(p + Pi - Math.atan2(A, B) * R2D), Math.asin(C) * R2D];
}

// ---- planets ---------------------------------------------------------------------

const ELEMENTS = {
  mercury: [[0.38709927, 0.20563593, 7.00497902, 252.25032350, 77.45779628, 48.33076593], [0.00000037, 0.00001906, -0.00594749, 149472.67411175, 0.16047689, -0.12534081]],
  venus: [[0.72333566, 0.00677672, 3.39467605, 181.97909950, 131.60246718, 76.67984255], [0.00000390, -0.00004107, -0.00078890, 58517.81538729, 0.00268329, -0.27769418]],
  emb: [[1.00000261, 0.01671123, -0.00001531, 100.46457166, 102.93768193, 0.0], [0.00000562, -0.00004392, -0.01294668, 35999.37244981, 0.32327364, 0.0]],
  mars: [[1.52371034, 0.09339410, 1.84969142, -4.55343205, -23.94362959, 49.55953891], [0.00001847, 0.00007882, -0.00813131, 19140.30268499, 0.44441088, -0.29257343]],
  uranus: [[19.18916464, 0.04725744, 0.77263783, 313.23810451, 170.95427630, 74.01692503], [-0.00196176, -0.00004397, -0.00242939, 428.48202785, 0.40805281, 0.04240589]],
  neptune: [[30.06992276, 0.00859048, 1.77004347, -55.12002969, 44.96476227, 131.78422574], [0.00026291, 0.00005105, 0.00035372, 218.45945325, -0.32241464, -0.00508664]],
  pluto: [[39.48211675, 0.24882730, 17.14001206, 238.92903833, 224.06891629, 110.30393684], [-0.00031596, 0.00005170, 0.00004818, 145.20780515, -0.04062942, -0.01183482]],
};
function kepler([e0, r], T) {
  const a = e0[0] + r[0] * T, e = e0[1] + r[1] * T, I = e0[2] + r[2] * T;
  const L = e0[3] + r[3] * T, vp = e0[4] + r[4] * T, Om = e0[5] + r[5] * T;
  const w = vp - Om;
  let M = norm360(L - vp); if (M > 180) M -= 360;
  const estar = e * R2D;
  let E = M + estar * sind(M);
  for (let i = 0; i < 30; i++) {
    const dE = (M - (E - estar * sind(E))) / (1 - e * cosd(E));
    E += dE;
    if (Math.abs(dE) <= 1e-8) break;
  }
  const xp = a * (cosd(E) - e), yp = a * Math.sqrt(1 - e * e) * sind(E);
  const cw = cosd(w), sw = sind(w), cO = cosd(Om), sO = sind(Om), cI = cosd(I), sI = sind(I);
  return [(cw * cO - sw * sO * cI) * xp + (-sw * cO - cw * sO * cI) * yp, (cw * sO + sw * cO * cI) * xp + (-sw * sO + cw * cO * cI) * yp, (sw * sI) * xp + (cw * sI) * yp];
}
const SCHLYTER = {
  jupiter: (d) => [100.4542 + 2.76854e-5 * d, 1.3030 - 1.557e-7 * d, 273.8777 + 1.64505e-5 * d, 5.20256, 0.048498 + 4.469e-9 * d, 19.8950 + 0.0830853001 * d],
  saturn: (d) => [113.6634 + 2.38980e-5 * d, 2.4886 - 1.081e-7 * d, 339.3939 + 2.97661e-5 * d, 9.55475, 0.055546 - 9.499e-9 * d, 316.9670 + 0.0334442282 * d],
};
function schlyter(name, T) {
  const d = T * 36525 + 1.5;
  const [N, i, w, a, e, M0] = SCHLYTER[name](d);
  const M = norm360(M0);
  let E = M + e * R2D * sind(M) * (1 + e * cosd(M));
  for (let k = 0; k < 30; k++) { const dE = (E - e * R2D * sind(E) - M) / (1 - e * cosd(E)); E -= dE; if (Math.abs(dE) < 1e-9) break; }
  const xv = a * (cosd(E) - e), yv = a * Math.sqrt(1 - e * e) * sind(E);
  const v = Math.atan2(yv, xv) * R2D, r = Math.hypot(xv, yv);
  const xh = r * (cosd(N) * cosd(v + w) - sind(N) * sind(v + w) * cosd(i));
  const yh = r * (sind(N) * cosd(v + w) + cosd(N) * sind(v + w) * cosd(i));
  const zh = r * sind(v + w) * sind(i);
  let lon = Math.atan2(yh, xh) * R2D, lat = Math.atan2(zh, Math.hypot(xh, yh)) * R2D;
  const Mj = norm360(SCHLYTER.jupiter(d)[5]), Ms = norm360(SCHLYTER.saturn(d)[5]);
  if (name === "jupiter") lon += -0.332 * sind(2 * Mj - 5 * Ms - 67.6) - 0.056 * sind(2 * Mj - 2 * Ms + 21) + 0.042 * sind(3 * Mj - 5 * Ms + 21) - 0.036 * sind(Mj - 2 * Ms) + 0.022 * cosd(Mj - Ms) + 0.023 * sind(2 * Mj - 3 * Ms + 52) - 0.016 * sind(Mj - 5 * Ms - 69);
  else {
    lon += 0.812 * sind(2 * Mj - 5 * Ms - 67.6) - 0.229 * cosd(2 * Mj - 4 * Ms - 2) + 0.119 * sind(Mj - 2 * Ms - 3) + 0.046 * sind(2 * Mj - 6 * Ms - 69) + 0.014 * sind(Mj - 3 * Ms + 32);
    lat += -0.020 * cosd(2 * Mj - 4 * Ms - 2) + 0.018 * sind(2 * Mj - 6 * Ms - 49);
  }
  lon -= (5029.0966 * T + 1.11113 * T * T) / 3600;
  return [r * cosd(lat) * cosd(lon), r * cosd(lat) * sind(lon), r * sind(lat)];
}
const helio = (name, T) => (name === "jupiter" || name === "saturn" ? schlyter(name, T) : kepler(ELEMENTS[name], T));
const hypot3 = (v) => Math.sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);

function sunFromEarth(earth, T) {
  const lon0 = norm360(Math.atan2(-earth[1], -earth[0]) * R2D);
  const lat0 = Math.atan2(-earth[2], Math.hypot(earth[0], earth[1])) * R2D;
  const [lon, lat] = precessFromJ2000(lon0, lat0, T);
  return [lon, lat, hypot3(earth)];
}
export function sunApparent(jde) {
  const T = centuries(jde);
  const [lon, lat, R] = sunFromEarth(kepler(ELEMENTS.emb, T), T);
  return { lon: norm360(lon + nutationLon(T) - 20.4898 / 3600 / R), lat, dist: R };
}
function planetApparent(name, jde) {
  const T = centuries(jde);
  const earth = kepler(ELEMENTS.emb, T);
  let tau = 0, g = [0, 0, 0];
  for (let i = 0; i < 3; i++) {
    const p = helio(name, (jde - tau - 2451545.0) / 36525);
    g = [p[0] - earth[0], p[1] - earth[1], p[2] - earth[2]];
    tau = hypot3(g) * 0.0057755183;
  }
  const [lon, lat] = precessFromJ2000(norm360(Math.atan2(g[1], g[0]) * R2D), Math.atan2(g[2], Math.hypot(g[0], g[1])) * R2D, T);
  const [sunTrue] = sunFromEarth(earth, T);
  const kappa = 20.49552 / 3600;
  const eE = 0.016708634 - 0.000042037 * T - 0.0000001267 * T * T;
  const piE = 102.93735 + 1.71946 * T + 0.00046 * T * T;
  const dLon = (-kappa * cosd(sunTrue - lon) + eE * kappa * cosd(piE - lon)) / cosd(lat);
  return { lon: norm360(lon + dLon + nutationLon(T)), lat, dist: hypot3(g) };
}

// ---- Moon (Meeus ch. 47) -------------------------------------------------------------

// [D, M, M', F, sum_l, sum_r]
const MOON_LR = [
  [0, 0, 1, 0, 6288774, -20905355], [2, 0, -1, 0, 1274027, -3699111], [2, 0, 0, 0, 658314, -2955968], [0, 0, 2, 0, 213618, -569925],
  [0, 1, 0, 0, -185116, 48888], [0, 0, 0, 2, -114332, -3149], [2, 0, -2, 0, 58793, 246158], [2, -1, -1, 0, 57066, -152138],
  [2, 0, 1, 0, 53322, -170733], [2, -1, 0, 0, 45758, -204586], [0, 1, -1, 0, -40923, -129620], [1, 0, 0, 0, -34720, 108743],
  [0, 1, 1, 0, -30383, 104755], [2, 0, 0, -2, 15327, 10321], [0, 0, 1, 2, -12528, 0], [0, 0, 1, -2, 10980, 79661],
  [4, 0, -1, 0, 10675, -34782], [0, 0, 3, 0, 10034, -23210], [4, 0, -2, 0, 8548, -21636], [2, 1, -1, 0, -7888, 24208],
  [2, 1, 0, 0, -6766, 30824], [1, 0, -1, 0, -5163, -8379], [1, 1, 0, 0, 4987, -16675], [2, -1, 1, 0, 4036, -12831],
  [2, 0, 2, 0, 3994, -10445], [4, 0, 0, 0, 3861, -11650], [2, 0, -3, 0, 3665, 14403], [0, 1, -2, 0, -2689, -7003],
  [2, 0, -1, 2, -2602, 0], [2, -1, -2, 0, 2390, 10056], [1, 0, 1, 0, -2348, 6322], [2, -2, 0, 0, 2236, -9884],
  [0, 1, 2, 0, -2120, 5751], [0, 2, 0, 0, -2069, 0], [2, -2, -1, 0, 2048, -4950], [2, 0, 1, -2, -1773, 4130],
  [2, 0, 0, 2, -1595, 0], [4, -1, -1, 0, 1215, -3958], [0, 0, 2, 2, -1110, 0], [3, 0, -1, 0, -892, 3258],
  [2, 1, 1, 0, -810, 2616], [4, -1, -2, 0, 759, -1897], [0, 2, -1, 0, -713, -2117], [2, 2, -1, 0, -700, 2354],
  [2, 1, -2, 0, 691, 0], [2, -1, 0, -2, 596, 0], [4, 0, 1, 0, 549, -1423], [0, 0, 4, 0, 537, -1117],
  [4, -1, 0, 0, 520, -1571], [1, 0, -2, 0, -487, -1739], [2, 1, 0, -2, -399, 0], [0, 0, 2, -2, -381, -4421],
  [1, 1, 1, 0, 351, 0], [3, 0, -2, 0, -340, 0], [4, 0, -3, 0, 330, 0], [2, -1, 2, 0, 327, 0],
  [0, 2, 1, 0, -323, 1165], [1, 1, -1, 0, 299, 0], [2, 0, 3, 0, 294, 0], [2, 0, -1, -2, 0, 8752],
];
// [D, M, M', F, sum_b]
const MOON_B = [
  [0, 0, 0, 1, 5128122], [0, 0, 1, 1, 280602], [0, 0, 1, -1, 277693], [2, 0, 0, -1, 173237], [2, 0, -1, 1, 55413],
  [2, 0, -1, -1, 46271], [2, 0, 0, 1, 32573], [0, 0, 2, 1, 17198], [2, 0, 1, -1, 9266], [0, 0, 2, -1, 8822],
  [2, -1, 0, -1, 8216], [2, 0, -2, -1, 4324], [2, 0, 1, 1, 4200], [2, 1, 0, -1, -3359], [2, -1, -1, 1, 2463],
  [2, -1, 0, 1, 2211], [2, -1, -1, -1, 2065], [0, 1, -1, -1, -1870], [4, 0, -1, -1, 1828], [0, 1, 0, 1, -1794],
  [0, 0, 0, 3, -1749], [0, 1, -1, 1, -1565], [1, 0, 0, 1, -1491], [0, 1, 1, 1, -1475], [0, 1, 1, -1, -1410],
  [0, 1, 0, -1, -1344], [1, 0, 0, -1, -1335], [0, 0, 3, 1, 1107], [4, 0, 0, -1, 1021], [4, 0, -1, 1, 833],
  [0, 0, 1, -3, 777], [4, 0, -2, 1, 671], [2, 0, 0, -3, 607], [2, 0, 2, -1, 596], [2, -1, 1, -1, 491],
  [2, 0, -2, 1, -451], [0, 0, 3, -1, 439], [2, 0, 2, 1, 422], [2, 0, -3, -1, 421], [2, 1, -1, 1, -366],
  [2, 1, 0, 1, -351], [4, 0, 0, 1, 331], [2, -1, 1, 1, 315], [2, -2, 0, -1, 302], [0, 0, 1, 3, -283],
  [2, 1, 1, -1, -229], [1, 1, 0, -1, 223], [1, 1, 0, 1, 223], [0, 1, -2, -1, -220], [2, 1, -1, -1, -220],
  [1, 0, 1, 1, -185], [2, -1, -2, -1, 181], [0, 1, 2, 1, -177], [4, 0, -2, -1, 176], [4, -1, -1, -1, 166],
  [1, 0, 1, -1, -164], [4, 0, 1, -1, 132], [1, 0, -1, -1, -119], [4, -1, 0, -1, 115], [2, -2, 0, 1, 107],
];
function moonArgs(T) {
  return [
    218.3164477 + 481267.88123421 * T - 0.0015786 * T * T + T ** 3 / 538841 - T ** 4 / 65194000,
    297.8501921 + 445267.1114034 * T - 0.0018819 * T * T + T ** 3 / 545868 - T ** 4 / 113065000,
    357.5291092 + 35999.0502909 * T - 0.0001536 * T * T + T ** 3 / 24490000,
    134.9633964 + 477198.8675055 * T + 0.0087414 * T * T + T ** 3 / 69699 - T ** 4 / 14712000,
    93.2720950 + 483202.0175233 * T - 0.0036539 * T * T - T ** 3 / 3526000 + T ** 4 / 863310000,
    1 - 0.002516 * T - 0.0000074 * T * T,
  ];
}
const eFactor = (m, E) => (m === 0 ? 1 : m === 1 || m === -1 ? E : E * E);
export function moonApparent(jde) {
  const T = centuries(jde);
  const [Lp, D, M, Mp, F, E] = moonArgs(T);
  let sl = 0, sr = 0, sb = 0;
  for (const [d, m, mp, f, l, r] of MOON_LR) {
    const arg = d * D + m * M + mp * Mp + f * F, ef = eFactor(m, E);
    sl += l * ef * sind(arg);
    sr += r * ef * cosd(arg);
  }
  for (const [d, m, mp, f, b] of MOON_B) sb += b * eFactor(m, E) * sind(d * D + m * M + mp * Mp + f * F);
  const A1 = 119.75 + 131.849 * T, A2 = 53.09 + 479264.290 * T, A3 = 313.45 + 481266.484 * T;
  sl += 3958 * sind(A1) + 1962 * sind(Lp - F) + 318 * sind(A2);
  sb += -2235 * sind(Lp) + 382 * sind(A3) + 175 * sind(A1 - F) + 175 * sind(A1 + F) + 127 * sind(Lp - Mp) - 115 * sind(Lp + Mp);
  return { lon: norm360(Lp + sl / 1e6 + nutationLon(T)), lat: sb / 1e6, dist: 385000.56 + sr / 1000 };
}
/** The longitude alone (the backtest's fast path; same numbers). */
function moonLongitude(jde) {
  const T = centuries(jde);
  const [Lp, D, M, Mp, F, E] = moonArgs(T);
  let sl = 0;
  for (let i = 0; i < 59; i++) {
    const [d, m, mp, f, l] = MOON_LR[i];
    sl += l * eFactor(m, E) * sind(d * D + m * M + mp * Mp + f * F);
  }
  sl += 3958 * sind(119.75 + 131.849 * T) + 1962 * sind(Lp - F) + 318 * sind(53.09 + 479264.290 * T);
  return norm360(Lp + sl / 1e6 + nutationLon(T));
}

// ---- positions ------------------------------------------------------------------------

export const BODIES = ["sun", "moon", "mercury", "venus", "mars", "jupiter", "saturn", "uranus", "neptune", "pluto"];
export const SIGNS = ["aries", "taurus", "gemini", "cancer", "leo", "virgo", "libra", "scorpio", "sagittarius", "capricorn", "aquarius", "pisces"];
export const PHASES = ["new_moon", "waxing_crescent", "first_quarter", "waxing_gibbous", "full_moon", "waning_gibbous", "last_quarter", "waning_crescent"];
const signIndex = (lon) => Math.min(Math.floor(norm360(lon) / 30), 11);
const signOf = (lon) => SIGNS[signIndex(lon)];
const phaseOf = (elong) => PHASES[Math.floor(norm360(elong + 22.5) / 45) % 8];

/** Apparent geocentric ecliptic-of-date position at an instant (ms); distance in au, km for the Moon. */
export function position(body, ms) {
  const jde = jdeFromMs(ms);
  if (body === "sun") return sunApparent(jde);
  if (body === "moon") return moonApparent(jde);
  return planetApparent(body, jde);
}
/** Degrees per day, the central difference over +-6 h. */
export function speed(body, ms) {
  let d = position(body, ms + 6 * HOUR).lon - position(body, ms - 6 * HOUR).lon;
  if (d > 180) d -= 360;
  if (d < -180) d += 360;
  return d * 2;
}
/** Longitude to 2 decimals and the sign and degree of that rounded value. */
function place(lon) {
  let l = Math.round(lon * 100) / 100;
  if (l >= 360) l -= 360;
  const i = Math.min(Math.floor(l / 30), 11);
  return [l, SIGNS[i], round(l - 30 * i, 2)];
}
function bodyAt(body, ms) {
  const p = position(body, ms), v = speed(body, ms);
  const [longitude, sign, degree] = place(p.lon);
  return { body, longitude, sign, degree, speed: round(v, 3), retrograde: body !== "sun" && body !== "moon" && v < 0 };
}

// ---- phases, stations, ingresses ---------------------------------------------------------

const RATE = 12.190749;
export const elongation = (ms) => norm360(position("moon", ms).lon - position("sun", ms).lon);
export function illumination(ms) {
  const jde = jdeFromMs(ms);
  const s = sunApparent(jde), m = moonApparent(jde);
  const psi = Math.acos(cosd(m.lat) * cosd(m.lon - s.lon));
  const R = s.dist * 149597870.7;
  const i = Math.atan2(R * Math.sin(psi), m.dist - R * Math.cos(psi));
  return (1 + Math.cos(i)) / 2;
}
export function nextPhase(ms, target) {
  let t = ms + (norm360(target - elongation(ms)) / RATE) * DAY;
  for (let i = 0; i < 8; i++) {
    const step = ((norm360(target - elongation(t) + 180) - 180) / RATE) * DAY;
    t += step;
    if (Math.abs(step) < 500) break;
  }
  return t;
}
export function prevPhase(ms, target) {
  let t = nextPhase(ms - (norm360(elongation(ms) - target) / RATE + 2) * DAY, target);
  if (t > ms) t = nextPhase(t - 33 * DAY, target);
  return t;
}
export function stations(body, from, to, stepDays = 1) {
  const out = [];
  const step = stepDays * DAY;
  let t0 = from, v0 = speed(body, t0);
  for (let t1 = from + step; t1 <= to + step; t1 += step) {
    const v1 = speed(body, t1);
    if ((v0 < 0) !== (v1 < 0)) {
      let a = t0, b = t1, va = v0;
      for (let i = 0; i < 40 && b - a > 30e3; i++) {
        const m = (a + b) / 2, vm = speed(body, m);
        if ((va < 0) === (vm < 0)) { a = m; va = vm; } else b = m;
      }
      const at = (a + b) / 2;
      if (at >= from && at < to) out.push({ at, kind: v1 < 0 ? "station_retrograde" : "station_direct" });
    }
    t0 = t1; v0 = v1;
  }
  return out;
}
export function ingresses(body, from, to) {
  const out = [];
  const step = 6 * HOUR;
  let s0 = signIndex(position(body, from).lon);
  for (let t = from; t < to; t += step) {
    const end = Math.min(t + step, to);
    const s1 = signIndex(position(body, end).lon);
    if (s1 !== s0) {
      let lo = t, hi = end;
      for (let k = 0; k < 30; k++) { const mid = (lo + hi) / 2; if (signIndex(position(body, mid).lon) === s0) lo = mid; else hi = mid; }
      out.push({ at: (lo + hi) / 2, sign: SIGNS[s1] });
      s0 = s1;
    }
  }
  return out;
}
const PLANETS = ["mercury", "venus", "mars", "jupiter", "saturn", "uranus", "neptune", "pluto"];
export function upcoming(from, to) {
  const out = [];
  for (const [kind, target] of [["new_moon", 0], ["first_quarter", 90], ["full_moon", 180], ["last_quarter", 270]]) {
    let t = nextPhase(from, target);
    if (t < from) t = nextPhase(from + DAY, target);
    while (t < to) { out.push({ at: t, kind, body: "moon", sign: signOf(position("moon", t).lon) }); t = nextPhase(t + 25 * DAY, target); }
  }
  for (const b of PLANETS) for (const s of stations(b, from, to, 1)) out.push({ at: s.at, kind: s.kind, body: b, sign: signOf(position(b, s.at).lon) });
  for (const b of ["sun", ...PLANETS]) for (const s of ingresses(b, from, to)) out.push({ at: s.at, kind: "ingress", body: b, sign: s.sign });
  return out.sort((a, b) => a.at - b.at).map((e) => ({ ...e, at: wibIso(e.at) }));
}
export function mercury(ms) {
  const back = stations("mercury", ms - 130 * DAY, ms, 1);
  const fwd = stations("mercury", ms, ms + 260 * DAY, 1);
  const retro = speed("mercury", ms) < 0;
  const lastSR = [...back].reverse().find((s) => s.kind === "station_retrograde");
  const nextSD = fwd.find((s) => s.kind === "station_direct");
  const nextSR = fwd.find((s) => s.kind === "station_retrograde");
  const sdAfter = nextSR && fwd.find((s) => s.kind === "station_direct" && s.at > nextSR.at);
  return {
    retrograde: retro,
    retrograde_since: retro && lastSR ? wibIso(lastSR.at) : null,
    retrograde_until: retro && nextSD ? wibIso(nextSD.at) : null,
    next_retrograde: nextSR ? { from: wibIso(nextSR.at), until: sdAfter ? wibIso(sdAfter.at) : null } : null,
  };
}
/** Mercury's retrograde intervals [station retrograde, station direct) covering [from, to], 5-day scan. */
function mercuryIntervals(from, to) {
  const iv = [];
  let open = speed("mercury", from) < 0 ? from : null;
  for (const s of stations("mercury", from, to, 5)) {
    if (s.kind === "station_retrograde") open = s.at;
    else if (open != null) { iv.push([open, s.at]); open = null; }
  }
  if (open != null) iv.push([open, to]);
  return iv;
}

// ---- weton and shio --------------------------------------------------------------------

const DAYS_ID = ["Minggu", "Senin", "Selasa", "Rabu", "Kamis", "Jumat", "Sabtu"];
const DAYS_EN = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const NEPTU_DAY = [5, 4, 3, 7, 8, 6, 9];
export const PASARAN = ["Legi", "Pahing", "Pon", "Wage", "Kliwon"];
const NEPTU_PASARAN = [5, 9, 7, 4, 8];
/** Julian day number of a civil date (17 Aug 1945 = 2431685, Jumat Legi). */
const jdn = (iso) => Math.round(jdFromMs(utcMs(iso, 12)));
const pasaranIndex = (iso) => ((jdn(iso) % 5) + 5) % 5;
export function weton(iso) {
  const n = jdn(iso), dow = (n + 1) % 7, p = ((n % 5) + 5) % 5;
  return { day: DAYS_ID[dow], day_en: DAYS_EN[dow], pasaran: PASARAN[p], neptu_day: NEPTU_DAY[dow], neptu_pasaran: NEPTU_PASARAN[p], neptu: NEPTU_DAY[dow] + NEPTU_PASARAN[p] };
}
const CST = 8 * HOUR;
const dayCst = (ms) => Math.floor((ms + CST) / DAY);
function sunLongitudeTime(ms, target) {
  let t = ms + (norm360(target - position("sun", ms).lon) / 0.98564736) * DAY;
  for (let i = 0; i < 10; i++) {
    const step = ((norm360(target - position("sun", t).lon + 180) - 180) / 0.98564736) * DAY;
    t += step;
    if (Math.abs(step) < 1000) break;
  }
  return t;
}
const monthStartOnOrBefore = (day) => dayCst(prevPhase(day * DAY - CST + DAY - 1, 0));
/** Imlek of a Gregorian year (China Standard Time date), from new moons and solar terms. */
export function chineseNewYear(year) {
  const ws1 = dayCst(sunLongitudeTime(Date.UTC(year - 1, 11, 1), 270));
  const ws2 = dayCst(sunLongitudeTime(Date.UTC(year, 11, 1), 270));
  const m11a = monthStartOnOrBefore(ws1), m11b = monthStartOnOrBefore(ws2);
  const starts = [m11a];
  let t = m11a * DAY - CST + DAY;
  for (;;) {
    const nm = nextPhase(t + 2 * DAY, 0), d = dayCst(nm);
    if (d > m11b) break;
    starts.push(d); t = nm;
    if (d === m11b) break;
  }
  let leap = -1;
  if (starts.length - 1 === 13) {
    for (let i = 1; i < starts.length - 1; i++) {
      const startMs = starts[i] * DAY - CST;
      const next = Math.ceil(position("sun", startMs).lon / 30) * 30;
      if (dayCst(sunLongitudeTime(startMs, next % 360)) >= starts[i + 1]) { leap = i; break; }
    }
  }
  return isoOfDay(starts[leap === 1 || leap === 2 ? 3 : 2]);
}
const ANIMALS = ["rat", "ox", "tiger", "rabbit", "dragon", "snake", "horse", "goat", "monkey", "rooster", "dog", "pig"];
const ANIMALS_ID = ["Tikus", "Kerbau", "Macan", "Kelinci", "Naga", "Ular", "Kuda", "Kambing", "Monyet", "Ayam", "Anjing", "Babi"];
const STEMS = ["wood", "wood", "fire", "fire", "earth", "earth", "metal", "metal", "water", "water"];
export function shio(iso) {
  const y = Number(iso.slice(0, 4));
  if (y < 1924 || y > 2050) return null;
  let ly = y, start = chineseNewYear(y);
  if (iso < start) { ly = y - 1; if (ly < 1924) return null; start = chineseNewYear(ly); }
  const a = (((ly - 4) % 12) + 12) % 12, s = (((ly - 4) % 10) + 10) % 10;
  return { animal: ANIMALS[a], animal_id: ANIMALS_ID[a], element: STEMS[s], year_start: start };
}

// ---- natal chart ----------------------------------------------------------------------------

export const NATAL_DATES = { launch: "1983-04-01", base: "1982-08-10" };
const ASPECTS = [["conjunction", 0], ["sextile", 60], ["square", 90], ["trine", 120], ["opposition", 180]];
export function natal(chart, ms) {
  const date = NATAL_DATES[chart];
  const at = noonMs(date);
  const bodies = BODIES.map((b) => ({ ...bodyAt(b, at), approximate: b === "moon" }));
  const transits = [];
  for (const t of BODIES.filter((b) => b !== "moon")) {
    const now = position(t, ms).lon, later = position(t, ms + DAY).lon;
    for (const n of bodies.filter((x) => x.body !== "moon")) {
      const sep = (lon) => { const d = Math.abs(norm360(lon - n.longitude)); return d > 180 ? 360 - d : d; };
      for (const [aspect, angle] of ASPECTS) {
        const orb = Math.abs(sep(now) - angle);
        if (orb <= 2) transits.push({ transit: t, aspect, natal: n.body, orb: round(orb, 2), applying: Math.abs(sep(later) - angle) < orb });
      }
    }
  }
  return { chart, date, at: wibIso(at), bodies, transits: transits.sort((a, b) => a.orb - b.orb) };
}

// ---- the sky for a date ----------------------------------------------------------------------

export const FIRST_DATE = "1800-01-01";
export const LAST_DATE = "2050-12-31";
export const METHOD = "Apparent geocentric positions in the tropical zodiac (ecliptic and equinox of date) at 16:00 WIB (UTC+7), the IDX close. Planets and the Earth from JPL's Keplerian elements (Standish, Table 1, 1800-2050; Pluto from JPL's archived table), Jupiter and Saturn from Schlyter's elements with the Jupiter-Saturn perturbations, the Moon from Meeus, Astronomical Algorithms, ch. 47; light time, aberration, nutation and precession applied; Delta T after Espenak and Meeus. Checked against JPL Horizons 1800-2050: Sun and Moon within 0.01°, planets within 0.04°; phase times within 2 minutes of USNO; stations within 3 hours; slow planets' ingresses within 12 hours, so one can land on the neighbouring date. Moon phase: eight 45° sectors of the Moon-Sun elongation centred on the exact phases. Weton: pasaran from the Julian day number (17 Aug 1945 = Jumat Legi) on the civil date (Javanese days begin at sunset). Shio: Imlek from new moons and solar terms in China Standard Time, checked against published dates 1924-2050. Natal chart at noon WIB, time unknown: no houses, the Moon approximate; its dates (launch 1983-04-01, base 1982-08-10) come from secondary sources.";
export const DISCLAIMER = "Traditional astrology, not a forecast or investment advice: the positions are astronomy, their meanings are tradition, and the backtest only describes the past.";

/** The idx.astro.v1 payload for a date (backtest null) and its warnings. The caller checks the range. */
export function sky(iso, days = 30, natalChart = "launch") {
  const ms = closeMs(iso);
  const from = dayStartMs(iso);
  let to = from + days * DAY;
  const warnings = [];
  const limit = dayStartMs(LAST_DATE) + DAY;
  if (to > limit) { to = limit; warnings.push(`events listed to ${LAST_DATE} only, the end of the planetary elements`); }
  const el = elongation(ms);
  const prevNew = prevPhase(ms, 0);
  const data = {
    date: iso,
    at: wibIso(ms),
    bodies: BODIES.map((b) => bodyAt(b, ms)),
    moon: {
      phase: phaseOf(el),
      elongation: round(el, 2) % 360,
      illumination: round(illumination(ms), 3),
      age_days: round((ms - prevNew) / DAY, 1),
      previous_new_moon: wibIso(prevNew),
      next_new_moon: wibIso(nextPhase(ms, 0)),
      next_full_moon: wibIso(nextPhase(ms, 180)),
    },
    mercury: mercury(ms),
    weton: weton(iso),
    shio: shio(iso),
    upcoming: { from: wibIso(from), to: wibIso(to), events: upcoming(from, to) },
    natal: natalChart === "none" ? null : natal(natalChart, ms),
    backtest: null,
    method: METHOD,
    disclaimer: DISCLAIMER,
  };
  return { data, warnings };
}

// ---- statistics (Numerical Recipes) ------------------------------------------------------------

function lnGamma(x) {
  const c = [76.18009172947146, -86.50532032941677, 24.01409824083091, -1.231739572450155, 0.1208650973866179e-2, -0.5395239384953e-5];
  let y = x, tmp = x + 5.5;
  tmp -= (x + 0.5) * Math.log(tmp);
  let ser = 1.000000000190015;
  for (const v of c) ser += v / ++y;
  return -tmp + Math.log((2.5066282746310005 * ser) / x);
}
function betaCf(a, b, x) {
  const FPMIN = 1e-300;
  const qab = a + b, qap = a + 1, qam = a - 1;
  let c = 1, d = 1 - (qab * x) / qap;
  if (Math.abs(d) < FPMIN) d = FPMIN;
  d = 1 / d;
  let h = d;
  for (let m = 1; m <= 300; m++) {
    const m2 = 2 * m;
    let aa = (m * (b - m) * x) / ((qam + m2) * (a + m2));
    d = 1 + aa * d; if (Math.abs(d) < FPMIN) d = FPMIN;
    c = 1 + aa / c; if (Math.abs(c) < FPMIN) c = FPMIN;
    d = 1 / d; h *= d * c;
    aa = (-(a + m) * (qab + m) * x) / ((a + m2) * (qap + m2));
    d = 1 + aa * d; if (Math.abs(d) < FPMIN) d = FPMIN;
    c = 1 + aa / c; if (Math.abs(c) < FPMIN) c = FPMIN;
    d = 1 / d;
    const del = d * c;
    h *= del;
    if (Math.abs(del - 1) < 3e-14) break;
  }
  return h;
}
function betaInc(a, b, x) {
  if (x <= 0) return 0;
  if (x >= 1) return 1;
  const bt = Math.exp(lnGamma(a + b) - lnGamma(a) - lnGamma(b) + a * Math.log(x) + b * Math.log(1 - x));
  return x < (a + 1) / (a + b + 2) ? (bt * betaCf(a, b, x)) / a : 1 - (bt * betaCf(b, a, 1 - x)) / b;
}
/** Two-sided p-value of Student's t with df degrees of freedom. */
export const pTwoSided = (t, df) => betaInc(df / 2, 0.5, df / (df + t * t));
const mean = (a) => a.reduce((s, x) => s + x, 0) / a.length;
const variance = (a, m) => a.reduce((s, x) => s + (x - m) ** 2, 0) / (a.length - 1);
const median = (a) => { const s = [...a].sort((x, y) => x - y); const n = s.length; return n % 2 ? s[(n - 1) / 2] : (s[n / 2 - 1] + s[n / 2]) / 2; };
export function welch(a, b) {
  if (a.length < 2 || b.length < 2) return null;
  const m1 = mean(a), m2 = mean(b), v1 = variance(a, m1) / a.length, v2 = variance(b, m2) / b.length;
  const se = Math.sqrt(v1 + v2);
  if (!(se > 0)) return null;
  const t = (m1 - m2) / se;
  const df = (v1 + v2) ** 2 / (v1 ** 2 / (a.length - 1) + v2 ** 2 / (b.length - 1));
  return { t, df, p: pTwoSided(t, df) };
}

// ---- backtest ---------------------------------------------------------------------------------

export const BACKTEST_METHOD = "Close-to-close daily returns of the index, each session assigned the factor values at 16:00 WIB of its date (the Moon's phase sector, Mercury retrograde or direct, the pasaran, the Sun's sign). Each group is compared with all other sessions by Welch's t (two-sided p from Student's t, Welch-Satterthwaite degrees of freedom), Bonferroni over all groups. Daily returns are fat-tailed and their volatility clusters, so p-values are approximate; Sun signs track calendar months and carry month-of-year effects. Descriptive statistics, not a trading signal.";

export class BacktestError extends Error {
  constructor(code, message) { super(message); this.code = code; }
}

/** Drops the last candle when it is today's session (Jakarta) before 17:00 WIB; returns its date or null. */
export function dropUnfinished(history, nowMs = Date.now()) {
  const last = history.candles.at(-1);
  const wib = new Date(nowMs + WIB).toISOString();
  if (last && last.time.slice(0, 10) === wib.slice(0, 10) && Number(wib.slice(11, 13)) < 17) {
    history.candles = history.candles.slice(0, -1);
    return last.time.slice(0, 10);
  }
  return null;
}

/** The backtest over idx.history.v1 data (daily, oldest first). Mirrors analysis::astro::backtest. */
export function backtest(history, period) {
  const closes = [];
  for (const c of history.candles) {
    const d = c.time.slice(0, 10);
    if (closes.length && closes.at(-1)[0] === d) closes.at(-1)[1] = c.close;
    else closes.push([d, c.close]);
  }
  if (closes.length > 2) {
    const gaps = closes.slice(1).map((c, i) => (Date.parse(c[0]) - Date.parse(closes[i][0])) / DAY).sort((a, b) => a - b);
    if (history.interval !== "1d" || gaps[Math.floor(gaps.length / 2)] > 4) throw new BacktestError("provider", `history for ${history.symbol} came back coarser than daily (${history.interval} bars)`);
  }
  if (closes.length < 31) throw new BacktestError("not_found", `${history.symbol} has ${closes.length} daily closes; the backtest needs at least 31`);
  const iv = mercuryIntervals(closeMs(closes[0][0]) - 40 * DAY, closeMs(closes.at(-1)[0]) + 40 * DAY);
  let k = 0;
  const feats = closes.map(([d]) => {
    const ms = closeMs(d), jde = jdeFromMs(ms);
    const sun = sunApparent(jde).lon;
    while (k < iv.length && iv[k][1] <= ms) k++;
    return { moon_phase: phaseOf(norm360(moonLongitude(jde) - sun)), mercury: k < iv.length && iv[k][0] <= ms ? "retrograde" : "direct", pasaran: PASARAN[pasaranIndex(d)].toLowerCase(), sun_sign: SIGNS[signIndex(sun)] };
  });
  const rets = closes.slice(1).map((c, i) => ({ r: (c[1] / closes[i][1] - 1) * 100, f: feats[i + 1] }));
  const all = rets.map((x) => x.r);
  const ORDER = { moon_phase: PHASES, mercury: ["retrograde", "direct"], pasaran: PASARAN.map((p) => p.toLowerCase()), sun_sign: SIGNS };
  const factors = Object.entries(ORDER).map(([factor, names]) => ({
    factor,
    groups: names.map((group) => {
      const g = rets.filter((x) => x.f[factor] === group).map((x) => x.r);
      const rest = rets.filter((x) => x.f[factor] !== group).map((x) => x.r);
      const w = welch(g, rest);
      const n = g.length, m = n ? mean(g) : null;
      const row = {
        group, n,
        mean_pct: n ? round(m, 4) : null,
        median_pct: n ? round(median(g), 4) : null,
        stdev_pct: n > 1 ? round(Math.sqrt(variance(g, m)), 4) : null,
        win_rate_pct: n ? round((g.filter((x) => x > 0).length / n) * 100, 2) : null,
        t_stat: w ? round(w.t, 3) : null,
        p_value: w ? w.p : null,
        significant: false,
      };
      return row;
    }),
  }));
  const groups = factors.flatMap((f) => f.groups);
  const tests = groups.filter((g) => g.p_value != null).length;
  const threshold = 0.05 / Math.max(tests, 1);
  let significant = 0;
  for (const g of groups) {
    g.significant = g.p_value != null && g.p_value < threshold;
    if (g.significant) significant++;
    if (g.p_value != null) g.p_value = round(g.p_value, 6);
  }
  return {
    symbol: history.symbol, period,
    from: closes[1][0], to: closes.at(-1)[0],
    returns: all.length,
    mean_pct: round(mean(all), 4), median_pct: round(median(all), 4),
    win_rate_pct: round((all.filter((x) => x > 0).length / all.length) * 100, 2),
    tests, alpha: 0.05, bonferroni_alpha: round(threshold, 6), significant,
    factors,
    method: BACKTEST_METHOD,
  };
}
