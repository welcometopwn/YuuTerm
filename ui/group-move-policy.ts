export function normalizedGroup(value: string): string {
  const group = value.trim();
  if (
    new TextEncoder().encode(group).length > 64 ||
    /[\u0000-\u001f\u007f-\u009f]/u.test(group)
  )
    throw Error(
      "Group must fit 64 UTF-8 bytes and contain no control characters.",
    );
  return group;
}
