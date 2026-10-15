// The one door to pdf.js: a dynamic import, so pdf.js is a chunk of its own (see pdfjs.ts).

export type PdfLib = typeof import("./pdfjs");

let lib: Promise<PdfLib> | undefined;

/** pdf.js and the document cache, loaded the first time anything asks. */
export function pdfLib(): Promise<PdfLib> {
  lib ??= import("./pdfjs").catch((e: unknown) => {
    lib = undefined;
    throw e;
  });
  return lib;
}
