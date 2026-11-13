import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { pathToFileURL } from 'node:url';

export const VERSION = '0.58.2';
const clean = value => typeof value === 'string' && /^[A-Za-z0-9_.:+/@-]{1,200}$/.test(value) ? value : 'redacted';
export function scan(run, root, images) {
  if (!images.length) throw new Error('runtime image archives required');
  const version = run('trivy', ['--version']);
  if (version.status !== 0 || !version.stdout.startsWith(`Version: ${VERSION}\n`)) throw new Error('scanner version mismatch');
  const reports = [ ['dependencies', ['fs', root]], ...images.map(p => ['runtime-image', ['image', '--input', p]]) ];
  const findings = new Map();
  const coverage = new Set();
  for (const [area, args] of reports) {
    const result = run('trivy', [...args, '--scanners', 'vuln', '--format', 'json', '--skip-db-update', '--skip-java-db-update', '--offline-scan']);
    if (result.status !== 0) throw new Error(`scanner failed: ${area}`);
    let report;
    try { report = JSON.parse(result.stdout); } catch { throw new Error('invalid scanner JSON'); }
    if (report.SchemaVersion !== 2 || !Array.isArray(report.Results)) throw new Error('incomplete scanner report');
    for (const item of report.Results) {
      if (area === 'runtime-image') coverage.add(area);
      else if (item.Type === 'cargo') coverage.add('rust');
      else if (['npm', 'pnpm', 'yarn'].includes(item.Type)) coverage.add('web');
      if (item.Vulnerabilities !== undefined && !Array.isArray(item.Vulnerabilities)) throw new Error('invalid vulnerabilities');
      for (const v of item.Vulnerabilities ?? []) {
        const finding = { area, ecosystem: clean(item.Type), advisory: clean(v.VulnerabilityID), package: clean(v.PkgName), installed: clean(v.InstalledVersion), fixed: clean(v.FixedVersion), severity: clean(v.Severity) };
        if ([finding.advisory, finding.package, finding.installed].includes('redacted')) throw new Error('invalid advisory identity');
        const key = createHash('sha256').update(JSON.stringify([area, finding.ecosystem, finding.advisory, finding.package, finding.installed])).digest('hex');
        findings.set(key, { key, ...finding });
      }
    }
  }
  if (!['rust', 'web', 'runtime-image'].every(x => coverage.has(x))) throw new Error('incomplete coverage');
  return { schema: 1, scanner: `trivy@${VERSION}`, coverage: [...coverage].sort(), findings: [...findings.values()].sort((a,b) => a.key.localeCompare(b.key)) };
}

export function publish(report, run) {
  for (const finding of report.findings) {
    const label = `advisory:${finding.key}`;
    const existing = run('terrarium', ['card', 'ls', '--label', label, '--json']);
    if (existing.status !== 0) throw new Error('backlog lookup failed');
    const cards = JSON.parse(existing.stdout);
    if (!Array.isArray(cards)) throw new Error('invalid backlog lookup');
    if (cards.length) continue;
    const result = run('terrarium', ['card', 'new', `Security advisory ${finding.advisory} in ${finding.package}`, '--column', 'backlog', '--label', label, '--label', 'security', '--body', `> Advisory report only. No automatic upgrade or image rebuild.\n> ${JSON.stringify(finding)}`]);
    if (result.status !== 0) throw new Error('backlog creation failed');
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const run = (cmd, args) => spawnSync(cmd, args, { encoding: 'utf8', timeout: 300000, maxBuffer: 32 * 1024 * 1024 });
  try {
    const args = process.argv.slice(2);
    const publishing = args[0] === '--publish';
    if (publishing) args.shift();
    if (args.length < 2) throw new Error('usage: advisories.mjs [--publish] ROOT IMAGE_ARCHIVE...');
    const report = scan(run, args[0], args.slice(1));
    if (publishing) publish(report, run);
    process.stdout.write(`${JSON.stringify(report)}\n`);
  } catch {
    process.stdout.write('{"schema":1,"status":"failed"}\n');
    process.exitCode = 1;
  }
}
