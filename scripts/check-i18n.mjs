// Verifies that every locale has the same keys and the same {placeholders}.
// Run via `npm run i18n:check` (also part of CI).

import { readFileSync } from 'node:fs';

const localesDir = new URL('../src/locales/', import.meta.url);
const locales = ['en', 'ru'];

const dictionaries = Object.fromEntries(
  locales.map((lang) => [
    lang,
    JSON.parse(readFileSync(new URL(`${lang}.json`, localesDir), 'utf8')),
  ]),
);

const allKeys = new Set(locales.flatMap((lang) => Object.keys(dictionaries[lang])));
const placeholders = (text) =>
  [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort().join(',');

let errors = 0;

for (const key of allKeys) {
  for (const lang of locales) {
    if (!(key in dictionaries[lang])) {
      console.error(`missing ${lang}: ${key}`);
      errors++;
    }
  }
}

for (const key of Object.keys(dictionaries.en)) {
  if (!(key in dictionaries.ru)) continue;
  if (placeholders(dictionaries.en[key]) !== placeholders(dictionaries.ru[key])) {
    console.error(`placeholder mismatch: ${key}`);
    errors++;
  }
}

if (errors > 0) {
  console.error(`i18n check failed: ${errors} error(s)`);
  process.exit(1);
}

console.log(`i18n OK: ${allKeys.size} keys across ${locales.length} locales`);
