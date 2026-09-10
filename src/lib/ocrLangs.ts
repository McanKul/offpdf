/** Friendly names for common Tesseract language codes. Unknown codes stay raw. */

const LABELS: Record<string, string> = {
  afr: "Afrikaans",
  amh: "Amharic",
  ara: "Arabic",
  aze: "Azerbaijani",
  bel: "Belarusian",
  ben: "Bengali",
  bos: "Bosnian",
  bul: "Bulgarian",
  cat: "Catalan",
  ces: "Czech",
  cym: "Welsh",
  dan: "Danish",
  deu: "German",
  ell: "Greek",
  eng: "English",
  epo: "Esperanto",
  est: "Estonian",
  eus: "Basque",
  fas: "Persian",
  fin: "Finnish",
  fra: "French",
  gle: "Irish",
  glg: "Galician",
  heb: "Hebrew",
  hin: "Hindi",
  hrv: "Croatian",
  hun: "Hungarian",
  hye: "Armenian",
  ind: "Indonesian",
  isl: "Icelandic",
  ita: "Italian",
  jpn: "Japanese",
  kan: "Kannada",
  kat: "Georgian",
  kaz: "Kazakh",
  khm: "Khmer",
  kor: "Korean",
  lao: "Lao",
  lat: "Latin",
  lav: "Latvian",
  lit: "Lithuanian",
  mal: "Malayalam",
  mar: "Marathi",
  mkd: "Macedonian",
  mlt: "Maltese",
  mon: "Mongolian",
  msa: "Malay",
  mya: "Burmese",
  nep: "Nepali",
  nld: "Dutch",
  nor: "Norwegian",
  pol: "Polish",
  por: "Portuguese",
  ron: "Romanian",
  rus: "Russian",
  slk: "Slovak",
  slv: "Slovenian",
  spa: "Spanish",
  sqi: "Albanian",
  srp: "Serbian",
  swa: "Swahili",
  swe: "Swedish",
  tam: "Tamil",
  tel: "Telugu",
  tha: "Thai",
  tur: "Turkish",
  ukr: "Ukrainian",
  urd: "Urdu",
  uzb: "Uzbek",
  vie: "Vietnamese",
};

export function ocrLangLabel(code: string): string {
  return LABELS[code] ?? code;
}

/** Sort by code, strip duplicates, join with `+` (`["tur","eng"]` → `"eng+tur"`). */
export function joinOcrLangs(codes: string[]): string {
  return Array.from(new Set(codes)).sort().join("+");
}

const DEFAULT_SEARCH_OCR_PREFERRED = ["eng", "tur"];

/** Preferred ∩ installed, then `joinOcrLangs`. Empty intersection → `""`. */
export function pickSearchOcrLang(installed: string[], preferred?: string[]): string {
  const prefs = preferred ?? DEFAULT_SEARCH_OCR_PREFERRED;
  const have = new Set(installed);
  return joinOcrLangs(prefs.filter((code) => have.has(code)));
}

/** Toast copy when search OCR has no English or Turkish pack (no picker on that path). */
export function searchOcrEmptyToast(): { title: string; description: string } {
  return {
    title: "No English or Turkish pack",
    description:
      "Install tesseract-lang, or run OCR from the OCR tool to pick an installed language.",
  };
}

/** Installed codes minus Tesseract utility packs (`osd`/`equ`/`snum`); order preserved. */
export function documentOcrLangs(installed: string[]): string[] {
  const skip = new Set(["osd", "equ", "snum"]);
  return installed.filter((code) => !skip.has(code));
}
