// Exact reviewed SPDX expressions for this locked graph; unknowns fail closed.
import fs from "node:fs";
const allowed = new Set(["MIT", "Apache-2.0", "Apache-2.0 OR MIT", "BSD-2-Clause", "BSD-3-Clause", "ISC", "0BSD", "MPL-2.0", "(MIT OR CC0-1.0)", "(MIT AND Zlib)", "OFL-1.1"]);
const lock = JSON.parse(fs.readFileSync("package-lock.json", "utf8"));
const packages = [], skipped = [], findings = [];
for (const [path, metadata] of Object.entries(lock.packages)) {
  if (!path) continue;
  if (!fs.existsSync(`${path}/package.json`)) {
    const matches=(values,value)=>!values || (!values.includes(`!${value}`) && (!values.some(v=>!v.startsWith("!")) || values.includes(value)));
    if (!metadata.optional || (matches(metadata.os,process.platform)&&matches(metadata.cpu,process.arch))) findings.push({ path, reason: "Package for this host missing; run npm ci" });
    skipped.push({ path, reason: "Optional platform package not installed on this host" });
    continue;
  }
  const pkg = JSON.parse(fs.readFileSync(`${path}/package.json`, "utf8"));
  if(pkg.version!==metadata.version)findings.push({path,reason:"Installed version differs from lockfile; run npm ci"});
  const license = pkg.license ?? metadata.license ?? null;
  packages.push({ name: pkg.name, version: pkg.version, license });
  if (!allowed.has(license)) findings.push({ name: pkg.name, version: pkg.version, license, reason: "Unreviewed license expression" });
}
const report = { passed: findings.length === 0, scope: "Installed Windows npm runtime/build packages; optional other-platform packages explicitly listed separately", packages, skipped, findings };
process.stdout.write(JSON.stringify(report, null, 2) + "\n");
process.exitCode = report.passed ? 0 : 1;
