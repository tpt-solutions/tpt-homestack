/** Exported memory */
export declare const memory: WebAssembly.Memory;
/**
 * assembly/index/tpt_alloc
 * @param len `i32`
 * @returns `i32`
 */
export declare function tpt_alloc(len: number): number;
/**
 * assembly/index/tpt_entry
 * @param ptr `i32`
 * @param len `i32`
 * @returns `i64`
 */
export declare function tpt_entry(ptr: number, len: number): bigint;
/**
 * assembly/index/abortTrap
 * @param message `usize`
 * @param fileName `usize`
 * @param line `u32`
 * @param column `u32`
 */
export declare function abortTrap(message: number, fileName: number, line: number, column: number): void;
