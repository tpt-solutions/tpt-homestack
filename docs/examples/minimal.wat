(module
  ;; The only import the host provides to plugins: `tpt_log`.
  (import "env" "tpt_log" (func $log (param i32 i32)))

  ;; One 64 KiB page for static data and the bump heap; two pages leaves room.
  (memory (export "memory") 2)

  ;; Bump pointer for tpt_alloc. Starts above the static data at offset 0..255.
  (global $heap (mut i32) (i32.const 65536))

  (data (i32.const 0) "{\"status\":200,\"content_type\":\"text/plain\",\"body\":\"hello from wat\"}")
  (data (i32.const 200) "entered")

  ;; Called by the host before it writes the request JSON or a host result
  ;; into guest memory. Must be re-entrant: imports may call it mid-request.
  (func (export "tpt_alloc") (param $len i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $len)))
    (local.get $p))

  ;; Entry point. The request JSON is at (ptr, len); this example ignores it.
  ;; Returns (ptr << 32) | len for the response JSON at offset 0.
  (func (export "tpt_entry") (param $ptr i32) (param $len i32) (result i64)
    (call $log (i32.const 200) (i32.const 7))
    (i64.const 66))
)
