// Builds dist/licenses/THIRD-PARTY-NOTICES.txt: the licence texts and copyright notices of
// every third-party crate compiled into calliope-gui and every npm package bundled into its
// frontend, as MIT, Apache-2.0, BSD, MPL-2.0 and friends require when distributing binaries.
// Runs as a Vite plugin at `vite build` (dist/ is embedded in the binary), so the file is
// regenerated on every build and never goes stale. Checked by tests/frontend.rs.
import { execFileSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const templateDir = join(here, 'licence-templates');
const OUT = 'licenses/THIRD-PARTY-NOTICES.txt';

/** One third-party package and the licence files it ships. */
interface Package {
  kind: 'crate' | 'npm';
  name: string;
  version: string;
  licence: string;
  source: string;
  files: { name: string; text: string }[];
  /** Set when the package ships no licence file and a standard text is used instead. */
  note?: string;
}

const LICENCE_FILE = /^(licen[cs]e|copying|notice|unlicense|copyright)/i;

function licenceFiles(dir: string, extra?: string | null): { name: string; text: string }[] {
  const names = readdirSync(dir).filter((f) => LICENCE_FILE.test(f) && statSync(join(dir, f)).isFile());
  if (extra && !names.includes(extra) && existsSync(join(dir, extra))) names.push(extra);
  return names.sort().map((name) => ({ name, text: readFileSync(join(dir, name), 'utf8').trim() }));
}

/** Standard text for a package that ships no licence file, with its authors as copyright holders. */
function standardText(expression: string, holders: string): { name: string; text: string } {
  const ids = expression.split(/\s+OR\s+|\s+AND\s+|\/|[()]/).map((s) => s.trim()).filter(Boolean);
  if (/\sAND\s/.test(expression)) throw new Error(`no licence file and an AND expression: ${expression}`);
  const preferred = ['Apache-2.0', 'MIT', 'BSD-3-Clause', 'MPL-2.0'].find((id) => ids.includes(id));
  if (!preferred) throw new Error(`no licence file and no template for ${expression}; add one to licence-templates/`);
  const text =
    preferred === 'Apache-2.0'
      ? readFileSync(join(here, '../../LICENSE'), 'utf8')
      : readFileSync(join(templateDir, `${preferred}.txt`), 'utf8')
          .replace('<year> <copyright holders>', holders)
          .replace('<year> <owner>', holders);
  return { name: `${preferred} (standard text)`, text: text.trim() };
}

function withFallback(p: Package, holders: string): Package {
  if (p.files.length) return p;
  return {
    ...p,
    files: [standardText(p.licence, holders)],
    note: `No licence file in the published package; its declared licence applies (standard text below). Authors: ${holders}.`,
  };
}

interface CargoMeta {
  packages: {
    id: string;
    name: string;
    version: string;
    license: string | null;
    license_file: string | null;
    authors: string[];
    repository: string | null;
    manifest_path: string;
  }[];
  workspace_members: string[];
  resolve: { nodes: { id: string; deps: { pkg: string; dep_kinds: { kind: string | null }[] }[] }[] };
}

/** Third-party crates reachable from `root` through normal (not build or dev) dependencies. */
export function crates(root: string): Package[] {
  const host = /host: (\S+)/.exec(execFileSync('rustc', ['-vV'], { encoding: 'utf8' }))![1];
  const meta: CargoMeta = JSON.parse(
    execFileSync('cargo', ['metadata', '--format-version', '1', '--locked', '--filter-platform', host], {
      cwd: here,
      encoding: 'utf8',
      maxBuffer: 1 << 28,
    }),
  );
  const byId = new Map(meta.packages.map((p) => [p.id, p]));
  const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
  const start = meta.packages.find((p) => p.name === root && meta.workspace_members.includes(p.id));
  if (!start) throw new Error(`no workspace crate ${root}`);
  const seen = new Set<string>();
  const todo = [start.id];
  while (todo.length) {
    const id = todo.pop()!;
    if (seen.has(id)) continue;
    seen.add(id);
    for (const d of nodes.get(id)!.deps) if (d.dep_kinds.some((k) => k.kind === null)) todo.push(d.pkg);
  }
  return [...seen]
    .filter((id) => !meta.workspace_members.includes(id))
    .map((id) => byId.get(id)!)
    .map((p) => {
      const holders = p.authors.length ? p.authors.join(', ') : `the ${p.name} authors`;
      return withFallback(
        {
          kind: 'crate',
          name: p.name,
          version: p.version,
          licence: p.license ?? `see ${p.license_file}`,
          source: p.repository ?? `https://crates.io/crates/${p.name}`,
          files: licenceFiles(dirname(p.manifest_path), p.license_file),
        },
        holders,
      );
    });
}

/** The npm package directory a bundled module id lives in, or null for app code. */
export function npmPackageDir(id: string): string | null {
  const clean = id.replace(/^\0/, '').split('?')[0];
  const i = clean.lastIndexOf('/node_modules/');
  if (i < 0) return null;
  const rest = clean.slice(i + '/node_modules/'.length).split('/');
  const name = rest[0].startsWith('@') ? `${rest[0]}/${rest[1]}` : rest[0];
  return clean.slice(0, i + '/node_modules/'.length) + name;
}

function npmPackage(dir: string): Package {
  const pj = JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8'));
  const author = typeof pj.author === 'string' ? pj.author : pj.author?.name;
  const repo = typeof pj.repository === 'string' ? pj.repository : pj.repository?.url;
  return withFallback(
    {
      kind: 'npm',
      name: pj.name,
      version: pj.version,
      licence: pj.license ?? 'not declared, see its licence text',
      source: repo ?? `https://www.npmjs.com/package/${pj.name}`,
      files: licenceFiles(dir),
    },
    author ?? `the ${pj.name} authors`,
  );
}

/** Packages pulled in by bare `@import "pkg"` lines of a stylesheet (Tailwind resolves those itself). */
export function cssImports(cssFile: string): string[] {
  const css = readFileSync(cssFile, 'utf8');
  return [...css.matchAll(/^@import\s+["']([^"'./][^"']*)["']/gm)].map((m) => {
    const parts = m[1].split('/');
    return parts[0].startsWith('@') ? `${parts[0]}/${parts[1]}` : parts[0];
  });
}

/** Renders the notices file: a package index, then each distinct licence text once. */
export function render(packages: Package[]): string {
  const key = (p: Package) => `${p.kind} ${p.name} ${p.version}`;
  const sorted = [...new Map(packages.map((p) => [key(p), p])).values()].sort(
    (a, b) => a.kind.localeCompare(b.kind) || a.name.localeCompare(b.name) || a.version.localeCompare(b.version),
  );
  // Group identical texts (ignoring whitespace) so each one is printed once.
  const texts = new Map<string, { text: string; users: string[] }>();
  for (const p of sorted) {
    for (const f of p.files) {
      const norm = f.text.replace(/\s+/g, ' ');
      const entry = texts.get(norm) ?? { text: f.text, users: [] };
      entry.users.push(`${p.name} ${p.version} (${f.name})`);
      texts.set(norm, entry);
    }
  }
  const out: string[] = [
    'THIRD-PARTY SOFTWARE IN CALLIOPE',
    '',
    'Calliope is licensed under the Apache License 2.0. It includes the third-party',
    'software listed below, each under its own licence. Generated at build time from',
    'Cargo.lock and the bundled npm modules; the licence texts follow the list.',
    '',
  ];
  for (const [kind, title] of [
    ['crate', 'Rust crates compiled into calliope-gui'],
    ['npm', 'npm packages bundled into the user interface'],
  ] as const) {
    out.push(`== ${title} ==`, '');
    for (const p of sorted.filter((q) => q.kind === kind)) {
      out.push(`${p.name} ${p.version}  [${p.licence}]  ${p.source}`);
      if (p.note) out.push(`    ${p.note}`);
    }
    out.push('');
  }
  out.push('== Licence texts ==', '');
  for (const { text, users } of texts.values()) {
    out.push('-'.repeat(78), `Used by: ${users.join(', ')}`, '-'.repeat(78), '', text, '');
  }
  return out.join('\n');
}

/** Vite plugin emitting licenses/THIRD-PARTY-NOTICES.txt into the build output. */
export function thirdPartyNotices(opts: { crate: string; stylesheet: string }) {
  return {
    name: 'calliope-third-party-notices',
    apply: 'build' as const,
    generateBundle(this: {
      getModuleIds(): IterableIterator<string>;
      emitFile(f: { type: 'asset'; fileName: string; source: string }): void;
    }) {
      const dirs = new Set<string>();
      for (const id of this.getModuleIds()) {
        const dir = npmPackageDir(id);
        if (dir) dirs.add(dir);
      }
      for (const name of cssImports(opts.stylesheet)) dirs.add(join(here, 'node_modules', name));
      const packages = [...crates(opts.crate), ...[...dirs].map(npmPackage)];
      this.emitFile({ type: 'asset', fileName: OUT, source: render(packages) });
    },
  };
}
