// Handing a person a file, and a name that is safe to give it: the one place the pages that export do it.

/** Offer `value` as a JSON file download named `fileName`. */
export function saveJson(fileName: string, value: unknown): void {
  const url = URL.createObjectURL(new Blob([JSON.stringify(value, null, 2)], { type: "application/json" }));
  const a = document.createElement("a");
  a.href = url;
  a.download = fileName;
  a.click();
  URL.revokeObjectURL(url);
}

/** `name` reduced to what every file system takes, or `fallback` when nothing is left. */
export const safeName = (name: string, fallback: string): string => name.replace(/[^A-Za-z0-9._-]+/g, "_").replace(/^_+|_+$/g, "") || fallback;
