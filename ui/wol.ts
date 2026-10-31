export function validMac(value: string): boolean {
  if (!value) return true;
  if (
    !/^(?:[0-9a-f]{2}:){5}[0-9a-f]{2}$/i.test(value) &&
    !/^(?:[0-9a-f]{2}-){5}[0-9a-f]{2}$/i.test(value)
  )
    return false;
  const bytes = value.split(/[:-]/).map((s) => Number.parseInt(s, 16));
  return bytes.some(Boolean) && (bytes[0] & 1) === 0;
}
