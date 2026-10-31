import { test } from "node:test";
import assert from "node:assert/strict";
import {
  highlightCells,
  compileRegex,
  highlightViewport,
} from "../ui/highlight-policy.ts";
import type { Highlighting } from "../ui/types.ts";

const config: Highlighting = {
  enabled: true,
  ipv4: true,
  ipv6: true,
  ipv4_color: "#89b4fa",
  ipv6_color: "#89b4fa",
  rules: [
    { keywords: "error, failed", color: "#f38ba8" },
    { keywords: "error", color: "#ffffff" },
  ],
};
const cells = (text: string) =>
  Array.from(text, (chars) => ({ chars, width: 1, plain: true }));

test("soft wraps join literal, regex and IP matches and split decorations by physical row", () => {
  const rows = [
    { cells: cells("ER"), isWrapped: false },
    { cells: cells("ROR 192.0."), isWrapped: true },
    { cells: cells("2.1 failed"), isWrapped: true },
  ];
  assert.deepEqual(highlightViewport(rows, false, config), [
    [{ x: 0, width: 2, color: "#f38ba8" }],
    [
      { x: 4, width: 6, color: "#89b4fa" },
      { x: 0, width: 3, color: "#f38ba8" },
    ],
    [
      { x: 0, width: 3, color: "#89b4fa" },
      { x: 4, width: 6, color: "#f38ba8" },
    ],
  ]);
  const regex = {
    ...config,
    rules: [{ keywords: "ERR.OR", regex: true, color: "#ffffff" }],
  };
  assert.equal(
    highlightViewport(
      [
        { cells: cells("ERR"), isWrapped: false },
        { cells: cells("XOR"), isWrapped: true },
      ],
      false,
      regex,
    )[1][0].width,
    3,
  );
  assert.deepEqual(highlightViewport(rows, true, config), [[], [], []]);
  assert.deepEqual(
    highlightViewport(
      rows.map((r) => ({ ...r, isWrapped: true })),
      false,
      config,
    ),
    [[], [], []],
  );
  assert.deepEqual(
    highlightViewport(
      [
        { cells: cells("ER"), isWrapped: false },
        { cells: cells("ROR"), isWrapped: false },
      ],
      false,
      config,
    ),
    [[], []],
  );
});

test("wrapped tokens preserve ANSI styling, wide padding, Unicode and resource bounds", () => {
  const source = [
    { cells: cells("ER"), isWrapped: false },
    { cells: cells("ROR"), isWrapped: true },
  ];
  source[1].cells[0].plain = false;
  assert.deepEqual(highlightViewport(source, false, config), [[], []]);
  const wide = [
    {
      cells: [...cells("er"), { chars: "", width: 1, plain: true }],
      isWrapped: false,
    },
    {
      cells: [
        { chars: "界", width: 2, plain: true },
        { chars: "", width: 0, plain: true },
        ...cells("ror"),
      ],
      isWrapped: true,
    },
  ];
  const before = JSON.stringify(wide);
  const regex = {
    ...config,
    rules: [{ keywords: "er界ror", regex: true, color: "#ffffff" }],
  };
  assert.deepEqual(highlightViewport(wide, false, regex), [
    [{ x: 0, width: 2, color: "#ffffff" }],
    [{ x: 0, width: 5, color: "#ffffff" }],
  ]);
  assert.equal(JSON.stringify(wide), before);
  for (const size of [9, 101])
    assert.ok(
      highlightViewport(
        Array.from({ length: size }, (_, i) => ({
          cells: cells("error"),
          isWrapped: i > 0,
        })),
        false,
        config,
      ).every((r) => r.length === 0),
    );
  assert.deepEqual(
    highlightViewport(
      [{ cells: cells("error".repeat(49)), isWrapped: false }],
      false,
      config,
    ),
    [[]],
  );
});

test("wrapped IPv6 and combining/surrogate text retain exact cell coordinates", () => {
  const rows = [
    { cells: cells("2001:db8:"), isWrapped: false },
    { cells: cells(":1"), isWrapped: true },
  ];
  assert.deepEqual(highlightViewport(rows, false, config), [
    [{ x: 0, width: 9, color: "#89b4fa" }],
    [{ x: 0, width: 2, color: "#89b4fa" }],
  ]);
  const unicode = {
    ...config,
    rules: [{ keywords: "é界", regex: true, color: "#ffffff" }],
  };
  assert.deepEqual(
    highlightViewport(
      [
        { cells: [{ chars: "é", width: 1, plain: true }], isWrapped: false },
        {
          cells: [
            { chars: "界", width: 2, plain: true },
            { chars: "", width: 0, plain: true },
          ],
          isWrapped: true,
        },
      ],
      false,
      unicode,
    ),
    [
      [{ x: 0, width: 1, color: "#ffffff" }],
      [{ x: 0, width: 2, color: "#ffffff" }],
    ],
  );
});
test("literal rules and validated IPv4 match without changing terminal content", () => {
  const source = cells(
    "ERROR error_count terror failed 192.0.2.254 999.1.2.3 1.2.3.4.5",
  );
  const before = JSON.stringify(source);
  assert.deepEqual(highlightCells(source, config), [
    { x: 32, width: 11, color: "#89b4fa" },
    { x: 0, width: 5, color: "#f38ba8" },
    { x: 25, width: 6, color: "#f38ba8" },
  ]);
  assert.equal(JSON.stringify(source), before);
  assert.deepEqual(highlightCells(source, { ...config, enabled: false }), []);
});
test("ANSI-styled tokens are preserved; wide/combining/surrogate cells keep column offsets", () => {
  const source = [
    { chars: "界", width: 2, plain: true },
    { chars: "", width: 0, plain: true },
    { chars: "界", width: 2, plain: true },
    { chars: "", width: 0, plain: true },
    { chars: "e\u0301", width: 1, plain: true },
    ...cells(" ERROR failed"),
  ];
  source[8].plain = false;
  assert.deepEqual(highlightCells(source, config), [
    { x: 12, width: 6, color: "#f38ba8" },
  ]);
});
test("hostile rules/output remain bounded and regex syntax is treated as invalid literal input", () => {
  assert.deepEqual(
    highlightCells(cells("error"), {
      ...config,
      ipv4: false,
      rules: [{ keywords: "(a+)+$, .*", color: "#f38ba8" }],
    }),
    [],
  );
  assert.deepEqual(
    highlightCells(cells("error"), {
      ...config,
      ipv4: false,
      rules: [{ keywords: "error", color: "url(javascript:alert(1))" }],
    }),
    [],
  );
  assert.deepEqual(highlightCells(cells("error".repeat(1000)), config), []);
  assert.deepEqual(
    highlightCells(
      [{ chars: "e".repeat(5000), width: 1, plain: true }],
      config,
    ),
    [],
  );
  assert.ok(highlightCells(cells("error ".repeat(40)), config).length <= 64);
});

test("IPv6 compression, brackets, zones and IPv4-mapped addresses use an independently configurable color", () => {
  const text =
    "[2001:db8::1]:443 ::1 fe80::abcd%eth0 ::ffff:192.0.2.1 2001::db8::1 12:34:56 aa:bb:cc:dd:ee:ff";
  const result = highlightCells(cells(text), config);
  assert.deepEqual(
    result.map((s) => text.slice(s.x, s.x + s.width)),
    ["2001:db8::1", "::1", "fe80::abcd%eth0", "::ffff:192.0.2.1"],
  );
  assert.ok(result.every((s) => s.color === config.ipv4_color));
  const separate = highlightCells(cells("::1 192.0.2.1"), {
    ...config,
    ipv6_color: "#ffffff",
  });
  assert.deepEqual(
    separate.map((span) => span.color),
    ["#ffffff", config.ipv4_color],
  );
  assert.deepEqual(
    highlightCells(cells("2001:db8::1"), { ...config, ipv6: false }),
    [],
  );
});

test("linear-time regex rules preserve precedence, ANSI cells and Unicode column offsets", () => {
  const settings = {
    ...config,
    rules: [
      { regex: true, keywords: "warn[0-9]+|failed", color: "#ff0000" },
      { keywords: "failed", color: "#00ff00" },
    ],
  };
  assert.deepEqual(highlightCells(cells("WARN42 failed"), settings), [
    { x: 0, width: 6, color: "#ff0000" },
    { x: 7, width: 6, color: "#ff0000" },
  ]);
  const wide = [
    { chars: "界", width: 2, plain: true },
    { chars: "", width: 0, plain: true },
    ...cells(" WARN42"),
  ];
  assert.deepEqual(highlightCells(wide, settings), [
    { x: 3, width: 6, color: "#ff0000" },
  ]);
  wide[5].plain = false;
  assert.deepEqual(highlightCells(wide, settings), []);
  assert.doesNotThrow(() =>
    compileRegex("(a+)+$")
      .matcher("a".repeat(2000) + "!")
      .find(),
  );
  assert.throws(() => compileRegex("(?<=error)foo"));
  assert.throws(() => compileRegex("(a)\\1"));
  assert.throws(() => compileRegex("["));
  assert.throws(() => compileRegex("(?:a{100}){100}"));
  assert.deepEqual(
    highlightCells(cells("hello"), {
      ...config,
      rules: [{ regex: true, keywords: "^", color: "#ffffff" }],
    }),
    [],
  );
});
