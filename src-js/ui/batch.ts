// Sending many things to a route that holds a request to a small body: in groups a request can carry.

/** `items` in groups a request can carry: a group closes at `maxCount` items or `maxBytes` of text, and an item bigger
 *  than that goes alone (the server's own refusal then says so). */
export function batches<T>(items: readonly T[], maxCount = 50, maxBytes = 1_000_000): T[][] {
  const out: T[][] = [];
  let group: T[] = [];
  let bytes = 0;
  for (const item of items) {
    const size = JSON.stringify(item).length;
    if (group.length > 0 && (group.length >= maxCount || bytes + size > maxBytes)) {
      out.push(group);
      group = [];
      bytes = 0;
    }
    group.push(item);
    bytes += size;
  }
  if (group.length > 0) out.push(group);
  return out;
}
