# Ownership fixtures

Workbooks of KSEI's monthly "Pemegang Saham di Atas 1%" register (published by
IDX) and the IDX page that lists them. Real excerpts keep the source's cell
values; synthetic files say so in their first row and in their name.

| file | provenance | used for |
| --- | --- | --- |
| above1_20260831_excerpt.xlsx | Real excerpt of the 31 Aug 2026 register, copied byte for byte from the reference CLI 0xrsydn/idx-cli (MIT, `crates/idx-core/tests/fixtures/ksei_above1_20260831_excerpt.xlsx`) | parser, queries, CLI tests |
| above1_20260731_synthetic.xlsx | Synthetic previous month for the change tests | `changes`, `foreign` |
| above1_20261030_excerpt.xlsx | Real excerpt of the 30 Oct 2026 register (13 columns, `CONTROLLER_OR_AFFILIATE` after `INVESTOR_CLASSIFICATION`): 11 data rows of BBCA and ZYRX at their original sheet rows 708-711 and 7148-7154, header on row 5. Copied byte for byte from the reference CLI 0xrsydn/idx-cli (MIT, `crates/idx-core/tests/fixtures/ksei_above1_20261030_excerpt.xlsx`, SHA-256 `8c148f645e0a180782d81191f7ee19a4c437a7ecfef933d67e16092c776d6368`) | controller flag (#29), sheet rows |
| above1_mixed_dates_synthetic.xlsx | Synthetic: AAAA with 2 rows dated 2026-02-27 and 1 dated 2026-01-30, BBBB with 1 row dated 2026-02-27 (L/A `F`), CCCC with 1 row dated 2026-01-30 | rows dated otherwise (#21) |
| above1_text_numbers_synthetic.xlsx | Synthetic, as of 31 Mar 2026: issuer TXTA with eight lines whose numbers are text cells in Indonesian and English notation (`7,19`, `1.234.567`, `2.500.000,00`, `1,000`, `54,94%`); rows 12 (`100,5`) and 13 (`10,5` shares) must be skipped | text numbers (#22) |
| idx_page_excerpt.html | Trimmed excerpt of the idx.co.id "Data Kepemilikan Saham" page as of 2026-09-24 (the Nuxt payload that lists the monthly files) | `ownership discover` |

The synthetic workbooks were generated with a small stdlib-only Python writer
(inline strings, numeric cells, fixed zip timestamps so the bytes are stable);
calamine reads them like the real files.
