import { test } from "node:test";
import assert from "node:assert/strict";
import {
  memoryPair,
  networkRate,
  statisticsEnabled,
} from "../ui/statistics-format.ts";

test("memory values share the unit selected by total host RAM", () => {
  assert.equal(
    memoryPair(512 * 1024 ** 2, 3 * 1024 ** 3, "de-AT"),
    "0,50 GiB / 3,00 GiB",
  );
  assert.equal(
    memoryPair(64 * 1024 ** 2, 512 * 1024 ** 2, "en-US"),
    "64.00 MiB / 512.00 MiB",
  );
  assert.equal(memoryPair(0, 1024 ** 3, "de-AT"), "0,00 GiB / 1,00 GiB");
});
test("network units stay explicit and use decimal bytes versus bits", () => {
  assert.equal(networkRate(125_000, "Megabytes"), "0.13 MB/s");
  assert.equal(networkRate(125_000, "Kilobytes"), "125.0 kB/s");
  assert.equal(networkRate(125_000, "Megabits"), "1.00 Mbit/s");
  assert.equal(networkRate(null, "Megabytes"), "—");
  assert.equal(networkRate(0, "Megabytes"), "0.00 MB/s");
});
test("explicit session choice overrides either global default; null inherits", () => {
  assert.equal(statisticsEnabled(null, true), true);
  assert.equal(statisticsEnabled(null, false), false);
  assert.equal(statisticsEnabled(false, true), false);
  assert.equal(statisticsEnabled(true, false), true);
});
