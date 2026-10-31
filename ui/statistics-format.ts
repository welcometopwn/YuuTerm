import type { StatisticsSettings } from "./types.ts";

export function memoryPair(
  used: number,
  total: number,
  locale?: string,
): string {
  const gib = total >= 1024 ** 3;
  const unit = gib ? 1024 ** 3 : 1024 ** 2;
  const label = gib ? "GiB" : "MiB";
  const format = new Intl.NumberFormat(locale, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
    useGrouping: false,
  });
  return `${format.format(used / unit)} ${label} / ${format.format(total / unit)} ${label}`;
}

export function networkRate(
  bytes: number | null,
  unit: StatisticsSettings["network_unit"],
): string {
  if (bytes === null) return "—";
  switch (unit) {
    case "Megabytes":
      return `${(bytes / 1_000_000).toFixed(2)} MB/s`;
    case "Kilobytes":
      return `${(bytes / 1_000).toFixed(1)} kB/s`;
    case "Megabits":
      return `${((bytes * 8) / 1_000_000).toFixed(2)} Mbit/s`;
  }
}

export function statisticsEnabled(
  override: boolean | null,
  global: boolean,
): boolean {
  return override ?? global;
}
