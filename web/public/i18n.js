// The page's language and the words the scripts print. Each page is served in
// Bahasa Indonesia at its plain path and in English under /en/; the <html lang>
// of the page decides which words and which number style the scripts use.
// Outside a browser (the tests) the language is English.

export const LANG = String(globalThis.document?.documentElement?.lang || "en").toLowerCase().startsWith("id") ? "id" : "en";
export const ID = LANG === "id";
/** The Intl locale behind every number and date the pages print. */
export const LOCALE = ID ? "id-ID" : "en-US";

/** The same page in the other language: / and /en/, /terminal and /en/terminal. */
export function otherPath(path = globalThis.location?.pathname || "/") {
  if (path === "/en" || path.startsWith("/en/")) return path.slice(3) || "/";
  return path === "/" ? "/en/" : `/en${path}`;
}

/** A path on this page's side of the site: "/terminal" stays as is in Indonesian and gains /en in English. */
export const here = (path) => (ID ? path : path === "/" ? "/en/" : `/en${path}`);

export const DAYS = ID ? ["Minggu", "Senin", "Selasa", "Rabu", "Kamis", "Jumat", "Sabtu"] : ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
export const DAYS_SHORT = ID ? ["Min", "Sen", "Sel", "Rab", "Kam", "Jum", "Sab"] : ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
export const MONTHS = ID
  ? ["Januari", "Februari", "Maret", "April", "Mei", "Juni", "Juli", "Agustus", "September", "Oktober", "November", "Desember"]
  : ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
export const MONTHS_SHORT = ID
  ? ["Jan", "Feb", "Mar", "Apr", "Mei", "Jun", "Jul", "Agu", "Sep", "Okt", "Nov", "Des"]
  : ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** The separator in a clock time: 16.14 in Indonesian, 16:14 in English. */
export const TIME_SEP = ID ? "." : ":";

/** The regular market's phases by id, as the exchange names them in each language. */
export const PHASES = {
  "pre-open": ID ? "Pra-pembukaan" : "Pre-opening",
  "session-1": ID ? "Sesi I" : "Session I",
  break: ID ? "Istirahat" : "Break",
  "session-2": ID ? "Sesi II" : "Session II",
  "pre-close": ID ? "Pra-penutupan" : "Pre-closing",
  post: ID ? "Pasca-penutupan" : "Post-closing",
  closed: ID ? "Penutupan" : "Close",
};

/** Picks the word for this page's language: t("Tutup", "Closed"). */
export const t = (id, en) => (ID ? id : en);
