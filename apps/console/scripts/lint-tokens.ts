/**
 * Rejects raw design values inside .svelte <style> blocks.
 * Use tokens from src/styles/tokens.css instead.
 *
 * Allowed: 0, 1px borders via var(--border-w) (raw 1px also tolerated in
 * borders), percentages, unitless numbers, ch/fr/vw/vh/dvh/cqw, ms in
 * keyframe percentages, and anything inside var()/calc() of tokens.
 * Escape hatch for a deliberate one-off: put `/* token-ok *\/` on the line.
 *
 *   bun run lint:tokens          # report
 */
import { Glob } from 'bun';

const RULES: [RegExp, string][] = [
  [/#[0-9a-fA-F]{3,8}\b/, 'hex color'],
  [/\b(rgba?|hsla?)\(/, 'rgb/hsl color'],
  [/\boklch\(/, 'raw oklch (define a token)'],
  [/(?<![\w-])(?!0(px|rem|em)\b)\d*\.?\d+(px|rem|em)\b/, 'raw length (use --space-*/--radius-*/--control-h*/--icon-*)'],
  [/z-index:\s*-?\d+/, 'raw z-index (use --z-*)'],
  [/\b\d+(ms|s)\b(?![^;]*@keyframes)/, 'raw duration (use --dur-*)'],
  [/var\(--(brand-gradient|pop)\)/, 'removed token'],
];
// Lengths that are fine raw: hairlines and media queries.
const ALLOW_LINE = [/token-ok/, /@media/, /@container/, /^\s*\d+%\s*\{/, /border(-\w+)?:\s*1px/, /outline-offset/];

let total = 0;
const perFile: Record<string, number> = {};
for await (const file of new Glob('src/**/*.svelte').scan('.')) {
  const src = await Bun.file(file).text();
  const m = src.match(/<style[^>]*>([\s\S]*?)<\/style>/);
  if (!m) continue;
  const offset = src.slice(0, m.index! + m[0].indexOf(m[1])).split('\n').length - 1;
  m[1].split('\n').forEach((line, i) => {
    if (ALLOW_LINE.some((r) => r.test(line))) return;
    const code = line.replace(/\/\*.*?\*\//g, '');
    for (const [re, why] of RULES) {
      if (re.test(code)) {
        total++;
        perFile[file] = (perFile[file] ?? 0) + 1;
        if (!process.argv.includes('--summary')) console.log(`${file}:${offset + i + 1}  ${why}\n    ${line.trim()}`);
        break;
      }
    }
  });
}
if (process.argv.includes('--summary'))
  Object.entries(perFile).sort((a, b) => b[1] - a[1]).forEach(([f, n]) => console.log(String(n).padStart(4), f));
console.log(`\n${total} raw value(s)`);
process.exit(total ? 1 : 0);
