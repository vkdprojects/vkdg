# Contributing Translations (i18n)

VKDG Console uses [Paraglide JS](https://inlang.com/m/gerre34r/library-inlang-paraglideJs) for i18n. Message files live in `messages/`.

## Editing translations online

1. Go to [fink.inlang.com](https://fink.inlang.com)
2. Enter this repository's URL
3. Select your language
4. Fix or add translations
5. Submit a PR with **only** `messages/{lang}.json` changed

## Adding a new language

1. Add the language tag to `project.inlang/settings.json`:
   ```json
   "languageTags": ["en", "pt-BR", "es"]
   ```
2. Create `messages/es.json` with translations for every key in `messages/en.json`
3. Open a PR — CI will validate with `@inlang/cli validate`

## Editing locally

```bash
# Translate missing strings using the AI CLI (draft quality, review before merging)
npx @inlang/cli machine translate --project ./project.inlang

# Validate all message files
npx @inlang/cli validate --project ./project.inlang
```

## Rules

- `messages/en.json` is the source of truth — never delete a key from it without a code change
- Keys follow `namespace_description` naming (e.g. `nav_overview`, `common_save`)
- Do not edit files in `src/lib/paraglide/` — they are generated at build time
- Pluralization and interpolation use ICU syntax if needed; simple string values for everything currently present
