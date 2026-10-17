// Regenerate with Node 22+: node scripts/generate-emoji.mjs
// Unicode 17 supplies fully qualified sequences; CLDR 48 supplies names and unique keywords.
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const output = new URL('../assets/emoji/', import.meta.url);
const cldr = 'https://raw.githubusercontent.com/unicode-org/cldr-json/48.0.0/';
const cache = join(tmpdir(), 'tokativ-cldr-48');
await mkdir(cache, { recursive: true });
async function download(url) {
  const path = join(cache, encodeURIComponent(url));
  try { return await readFile(path, 'utf8'); } catch {}
  for (let attempt = 0; ; attempt++) {
    try {
      const response = await fetch(url, { signal: AbortSignal.timeout(30000) });
      if (!response.ok) throw new Error(`${response.status}: ${url}`);
      const text = await response.text();
      await writeFile(path, text);
      return text;
    } catch (error) { if (attempt >= 2) throw error; }
  }
}
// Split scripts without word spaces into individual characters, on both sides of a command.
// Keep combining marks in the other scripts, including Indic and Arabic names.
const scripts = ['Han', 'Hiragana', 'Katakana', 'Hangul', 'Thai', 'Lao', 'Khmer', 'Myanmar'];
const characters = scripts.map(script => `\\p{Script=${script}}`).join('');
const tokens = new RegExp(`[${characters}]|(?:(?![${characters}])[\\p{L}\\p{N}\\p{M}])+`, 'gu');
const normalize = text => (text.toLowerCase().replaceAll('ё', 'е')
  .replaceAll('#', ' number sign ').replaceAll('*', ' asterisk ')
  .match(tokens) ?? []).join(' ');
const unqualified = text => text.replaceAll('\uFE0F', '');
const names = new Map();
const sequences = new Map();
const keywords = new Map();
const stopWords = new Set(['emoji', 'emojis', 'эмодзи', 'эмоджи', 'and', 'the', 'for', 'with',
  'that', 'this', 'from', 'are', 'was', 'you', 'your', 'our', 'but', 'not',
  'или', 'для', 'это', 'как', 'что', 'его', 'она', 'они', 'мой', 'моя', 'мне']);
function name(text, emoji) {
  const key = normalize(text);
  if (key && !names.has(key)) names.set(key, emoji);
}
const test = await download('https://www.unicode.org/Public/17.0.0/emoji/emoji-test.txt');
for (const line of test.split('\n')) {
  const match = line.match(/^([A-F0-9 ]+)\s*;\s*fully-qualified\s*#\s*\S+\s+E[\d.]+\s+(.+)$/);
  if (!match) continue;
  const emoji = String.fromCodePoint(...match[1].trim().split(/\s+/).map(code => parseInt(code, 16)));
  sequences.set(unqualified(emoji), emoji);
  name(match[2], emoji);
}
const directories = await Promise.all(['cldr-annotations-full/annotations', 'cldr-annotations-derived-full/annotationsDerived']
  .map(async path => JSON.parse(await download(`https://api.github.com/repos/unicode-org/cldr-json/contents/cldr-json/${path}?ref=48.0.0`))));
const locales = directories.map(files => new Set(files.filter(file => file.type === 'dir'
  && (/^[a-z]{2,3}$/.test(file.name) || ['zh-Hant', 'pt-PT'].includes(file.name))).map(file => file.name)));
const languages = ['en', 'ru', ...[...locales[0]].filter(lang => !['en', 'ru', 'und'].includes(lang)).sort()];
const tasks = languages.flatMap(language => [false, true].filter(derived => locales[Number(derived)].has(language))
  .map(derived => ({ language, derived })));
let cursor = 0;
const annotations = new Map();
await Promise.all(Array.from({ length: 8 }, async () => {
  while (cursor < tasks.length) {
    const task = tasks[cursor++];
    const { language, derived } = task;
    const group = derived ? 'annotationsDerived' : 'annotations';
    const packageName = derived ? 'cldr-annotations-derived-full' : 'cldr-annotations-full';
    const data = JSON.parse(await download(`${cldr}cldr-json/${packageName}/${group}/${language}/annotations.json`));
    annotations.set(task, data[group].annotations ?? {});
  }
}));
// Apply in a fixed order even though downloads finish out of order.
for (const task of tasks) {
    const { language } = task;
    for (const [sequence, annotation] of Object.entries(annotations.get(task))) {
      const emoji = sequences.get(unqualified(sequence));
      if (!emoji) continue;
      for (const text of annotation.tts ?? []) {
        const conjunction = { en: 'and', ru: 'и', uk: 'і', es: 'y', fr: 'et', de: 'und',
          pt: 'e', it: 'e', nl: 'en', sv: 'och', no: 'og', da: 'og', fi: 'ja', pl: 'i', tr: 've' }[language];
        for (const variant of [text, conjunction ? text.replaceAll('&', ` ${conjunction} `) : text]) {
          name(variant, emoji);
          name(variant.replace(/^(flag|флаг):\s*/i, ''), emoji);
        }
      }
      for (const text of ['en', 'ru'].includes(language) ? annotation.default ?? [] : []) {
        const key = normalize(text);
        const values = keywords.get(key) ?? new Set();
        values.add(emoji);
        keywords.set(key, values);
      }
    }
}
// Search keywords shared by several emoji are deliberately excluded.
for (const [key, values] of keywords) {
  if (key.length >= 3 && !stopWords.has(key) && values.size === 1) name(key, [...values][0]);
}
// Common spoken names that differ from the formal Unicode/CLDR names.
for (const [emoji, aliases] of [
  ['👍', ['thumbs up', 'thumb up', 'thumbsup', 'палец вверх']],
  ['👎', ['thumbs down', 'thumb down', 'thumbsdown', 'палец вниз']],
  ['❤️', ['heart', 'сердце', 'сердечко', 'красное сердце']],
  ['😊', ['smile', 'smiley', 'smiling', 'улыбка', 'улыбающийся']],
  ['😂', ['tears of joy', 'смеется до слез', 'смех до слез']],
  ['😢', ['crying', 'плачущий']],
  ['🇺🇸', ['us flag', 'usa flag', 'флаг сша']],
  ['🇬🇧', ['uk flag', 'флаг великобритании']],
  ['🇷🇺', ['флаг россии']],
]) {
  for (const alias of aliases) names.set(normalize(alias), emoji);
}
// UTF-8 byte ordering matches Rust's str comparison, including supplementary characters.
const rows = [...names].sort(([a], [b]) => Buffer.compare(Buffer.from(a), Buffer.from(b)));
const maxWords = rows.reduce((maximum, [key]) => Math.max(maximum, key.split(' ').length), 0);
await mkdir(output, { recursive: true });
await writeFile(new URL('names.tsv', output), `# max_words ${maxWords}; Unicode Emoji 17.0 / CLDR 48; scripts/generate-emoji.mjs\n`
  + rows.map(([key, value]) => `${key}\t${value}`).join('\n') + '\n');
await writeFile(new URL('languages.txt', output), languages.join('\n') + '\n');
await writeFile(new URL('LICENSE', output), await download(`${cldr}LICENSE`));
console.log(`${languages.length} locales, ${sequences.size} emoji, ${rows.length} names and aliases`);
