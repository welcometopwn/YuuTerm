import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { pathToFileURL } from "node:url";

export function validateException(policy, lock, authSource, now = new Date()) {
  if (policy.advisory !== "RUSTSEC-2023-0071" || !/^\d{4}-\d{2}-\d{2}$/.test(policy.review_by) || !Number.isFinite(new Date(policy.review_by + "T00:00:00Z").getTime()))
    throw new Error("Invalid RSA exception policy/date");
  if (now >= new Date(policy.review_by + "T00:00:00Z"))
    throw new Error("RSA exception expired; review the advisory and call paths");
  for (const expected of policy.packages) {
    const blocks = lock.split("[[package]]").filter((b) => b.includes(`name = "${expected.name}"\n`));
    if (blocks.length !== 1 || !blocks[0].includes(`version = "${expected.version}"`) || !blocks[0].includes(`checksum = "${expected.checksum}"`))
      throw new Error("RSA exception package/version/checksum changed");
  }
  const hash = createHash("sha256").update(authSource.replaceAll("\r\n", "\n")).digest("hex");
  if (hash !== policy.authentication_sha256)
    throw new Error("RSA authentication source changed; review exposure again");
  const dependents = lock.split("[[package]]").filter((b) => /\n "rsa(?: [^"]+)?",?\n/.test(b)).map((b) => b.match(/name = "([^"]+)"/)[1]).sort();
  if (JSON.stringify(dependents) !== JSON.stringify(["russh", "ssh-key"]))
    throw new Error("RSA has an unreviewed direct dependent");
  return { advisory: policy.advisory, review_by: policy.review_by, status: "accepted-risk", private_key_files: "supported; in-process RSA-SHA2 signing", rsa_ciphertext_decryption: "no reviewed production call path" };
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const [policy, lock, auth] = await Promise.all([
      readFile("docs/evidence/rsa-exception.json", "utf8"), readFile("Cargo.lock", "utf8"), readFile("crates/yuuterm-ssh/src/ssh.rs", "utf8"),
    ]);
    console.log(JSON.stringify(validateException(JSON.parse(policy), lock.replaceAll("\r\n", "\n"), auth)));
  } catch (e) { console.error(e.message); process.exitCode = 1; }
}
