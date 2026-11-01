import test from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, existsSync, writeFileSync } from "node:fs";
import path from "node:path";
import os from "node:os";

const payload = Buffer.from("public download fixture\n");
const digest = createHash("sha512").update(payload).digest("hex");
const quote = (value) => `'${value.replaceAll("'", "''")}'`;

async function fixture(run) {
  const requests = new Map();
  const server = createServer((req, res) => {
    requests.set(req.url, (requests.get(req.url) ?? 0) + 1);
    if (req.url === "/stall.bin") {
      res.writeHead(200, { "content-length": 100 });
      res.flushHeaders();
      return;
    }
    if (req.url === "/retry.bin" && requests.get(req.url) === 1) {
      res.writeHead(503);
      res.end();
      return;
    }
    res.end(payload);
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  try {
    await run(`http://127.0.0.1:${server.address().port}`, requests);
  } finally {
    server.closeAllConnections();
    await new Promise((resolve) => server.close(resolve));
  }
}

function download(mirror, file, directory, checksum = digest, retries = 0) {
  const module = path.resolve("scripts/PinnedDownload.psm1");
  const script = `Import-Module ${quote(module)} -Force; Get-PinnedFile -Mirror ${quote(mirror)} -Item ([pscustomobject]@{path=${quote(file)};sha512=${quote(checksum)}}) -Directory ${quote(directory)} -TimeoutSeconds 1 -ConnectTimeoutSeconds 1 -Retries ${retries}`;
  return new Promise((resolve) => {
    const child = spawn(
      "pwsh",
      [
        "-NoProfile",
        "-EncodedCommand",
        Buffer.from(script, "utf16le").toString("base64"),
      ],
      { windowsHide: true },
    );
    let output = "";
    let timedOut = false;
    child.stdout.on("data", (chunk) => {
      output += chunk;
    });
    child.stderr.on("data", (chunk) => {
      output += chunk;
    });
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill();
    }, 6000);
    child.on("close", (code) => {
      clearTimeout(timer);
      resolve({ code, output, timedOut });
    });
  });
}

test("stalled response fails within the download deadline", async () =>
  fixture(async (url) => {
    const folder = mkdtempSync(path.join(os.tmpdir(), "yuuterm-download-"));
    const result = await download(url, "stall.bin", folder);
    assert.equal(
      result.timedOut,
      false,
      "download ignored its deadline and had to be killed",
    );
    assert.notEqual(result.code, 0);
    assert.equal(existsSync(path.join(folder, "stall.bin")), false);
  }));

test("transient HTTP failure is retried and verified bytes are cached", async () =>
  fixture(async (url, requests) => {
    const folder = mkdtempSync(path.join(os.tmpdir(), "yuuterm-download-"));
    const first = await download(url, "retry.bin", folder, digest, 1);
    assert.equal(first.code, 0, first.output);
    assert.equal(requests.get("/retry.bin"), 2);
    assert.deepEqual(readFileSync(path.join(folder, "retry.bin")), payload);
    const cached = await download(url, "retry.bin", folder);
    assert.equal(cached.code, 0, cached.output);
    assert.equal(
      requests.get("/retry.bin"),
      2,
      "valid cached bytes were downloaded again",
    );
    assert.match(cached.output, /Cache hit/);
  }));

test("bad content cannot replace a cached file", async () =>
  fixture(async (url) => {
    const folder = mkdtempSync(path.join(os.tmpdir(), "yuuterm-download-"));
    writeFileSync(path.join(folder, "wrong.bin"), "previous bytes");
    const result = await download(url, "wrong.bin", folder, "0".repeat(128));
    assert.notEqual(result.code, 0);
    assert.equal(
      readFileSync(path.join(folder, "wrong.bin"), "utf8"),
      "previous bytes",
    );
  }));
