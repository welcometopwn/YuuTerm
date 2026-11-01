import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtempSync } from "node:fs";
import path from "node:path";
import os from "node:os";

const quote = (value) => `'${value.replaceAll("'", "''")}'`;
const module = path.resolve("scripts/ArchiveTools.psm1");
function powershell(script) {
  return new Promise((resolve) => {
    const child = spawn(
      "pwsh",
      [
        "-NoProfile",
        "-OutputFormat",
        "Text",
        "-EncodedCommand",
        Buffer.from(
          `$ErrorActionPreference='Stop'; Import-Module ${quote(module)} -Force; ${script}`,
          "utf16le",
        ).toString("base64"),
      ],
      { windowsHide: true },
    );
    let output = "";
    child.stdout.on("data", (value) => {
      output += value;
    });
    child.stderr.on("data", (value) => {
      output += value;
    });
    const timer = setTimeout(() => child.kill(), 45000);
    child.on("close", (code) => {
      clearTimeout(timer);
      resolve({ code, output });
    });
  });
}

test("archive processes have a hard deadline", async () => {
  const result = await powershell(
    `$exe=(Get-Process -Id $PID).Path; try { Invoke-ArchiveProcess -FilePath $exe -ArgumentList @('-NoProfile','-Command','Start-Sleep -Seconds 30') -TimeoutSeconds 1; throw 'Deadline ignored' } catch { if ($_.Exception.Message -notmatch 'timed out after 1 seconds') { throw } }; Write-Output 'Expected timeout confirmed'`,
  );
  assert.equal(result.code, 0, result.output);
});

test("both process output streams are drained without a pipe deadlock", async () => {
  const result = await powershell(
    `$exe=(Get-Process -Id $PID).Path; $value=Invoke-ArchiveProcess -FilePath $exe -ArgumentList @('-NoProfile','-Command','[Console]::Error.Write([string]::new([char]120,200000)); [Console]::Write("complete")') -TimeoutSeconds 5; if ($value -ne 'complete') { throw 'Missing process output' }`,
  );
  assert.equal(result.code, 0, result.output);
});

test("the actual pinned Bash archive extracts without native link creation", async () => {
  const directory = mkdtempSync(path.join(os.tmpdir(), "yuuterm-archive-"));
  const download = path.resolve("scripts/PinnedDownload.psm1");
  const pin = path.resolve("scripts/x11-runtime.json");
  const result = await powershell(
    `Import-Module ${quote(download)} -Force; $pin=Get-Content ${quote(pin)} -Raw | ConvertFrom-Json; $item=($pin.packages | Where-Object name -eq 'bash').install; $archive=Get-PinnedFile -Mirror $pin.mirror -Item $item -Directory ${quote(path.resolve("target/x11-cache"))} -TimeoutSeconds 20 -Retries 0; $links=@(Expand-PinnedArchive -Archive $archive -Directory ${quote(directory)} -TimeoutSeconds 10); if (-not (Test-Path ${quote(path.join(directory, "usr/bin/bash.exe"))})) { throw 'Missing Bash executable' }; if (Test-Path ${quote(path.join(directory, "usr/bin/sh.exe"))}) { throw 'Native hardlink was unexpectedly created' }; if (-not ($links | Where-Object { $_.Name -eq 'usr/bin/sh.exe' -and $_.HardLink })) { throw 'Bash hardlink was not retained for explicit resolution' }`,
  );
  assert.equal(result.code, 0, result.output);
});
