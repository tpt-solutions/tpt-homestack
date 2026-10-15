async function instantiate(module, imports = {}) {
  const adaptedImports = {
    env: Object.assign(Object.create(globalThis), imports.env || {}, {
      tpt_log(ptr, len) {
        // assembly/index/tpt_log(usize, i32) => void
        ptr = ptr >>> 0;
        tpt_log(ptr, len);
      },
    }),
  };
  const { exports } = await WebAssembly.instantiate(module, adaptedImports);
  return exports;
}
export const {
  memory,
  tpt_alloc,
  tpt_entry,
  abortTrap,
} = await (async url => instantiate(
  await (async () => {
    const isNodeOrBun = typeof process != "undefined" && process.versions != null && (process.versions.node != null || process.versions.bun != null);
    if (isNodeOrBun) { return globalThis.WebAssembly.compile(await (await import("node:fs/promises")).readFile(url)); }
    else { return await globalThis.WebAssembly.compileStreaming(globalThis.fetch(url)); }
  })(), {
  }
))(new URL("plugin.wasm", import.meta.url));
