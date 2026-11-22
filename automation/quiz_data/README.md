# Quiz data

Used by `automation/quiz.py` to build the flag and capital quiz Shorts. There is no
research step and no Claude for quizzes: the questions come from these files only.

| File | Source | Licence |
|------|--------|---------|
| `countries.csv` | Wikidata, sovereign states (`wdt:P31 wd:Q3624078`) with ISO 3166-1 alpha-2 code (`P297`), capital (`P36`) and continent (`P30`), fetched 2026-11-22 | CC0 |
| `india_states.csv` | Wikidata, states of India (`wd:Q12443800`) and their current capitals, fetched 2026-11-22 | CC0 |
| flags (downloaded per video) | flagcdn.com (`https://flagcdn.com/w640/<iso2>.png`), the national flags from Wikimedia Commons | public domain |

Hand edits on top of Wikidata:

- Short display names: China, Netherlands, Bahamas, Gambia, Micronesia, Czechia, Ivory Coast.
- Denmark added (Wikidata files it under "Kingdom of Denmark", not as a sovereign state).
- Antigua and Barbuda capital filled in (St. John's); Nauru's is shown as Yaren.
- Multi-capital countries show every capital (`Sucre / La Paz`) and have
  `capital_quiz = 0`, as do disputed cases (Israel, Palestine, Taiwan, Kosovo), so
  they are never the answer to "guess the capital".
- Karnataka, Maharashtra and Uttarakhand have a second (winter/summer) seat; the
  main capital is used.
- `region` is the first Wikidata continent, except Russia (Europe), Turkey and
  Kazakhstan (Asia). The Americas are one region.
- `easy` (well-known flags), `island` (island nations, all of Oceania) and
  `lookalike` (groups of similar flags, used for the Look-Alikes quiz and for harder
  wrong answers) are the channel's own picks.

Re-check before relying on a capital that has recently moved.
