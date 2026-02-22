//! # LOS SDK — Smart Contract Development Kit for Unauthority
//!
//! This crate provides safe Rust wrappers around the UVM host functions,
//! allowing developers to write smart contracts that interact with the
//! LOS blockchain.
//!
//! ## Features
//! - `#![no_std]` — compiles to `wasm32-unknown-unknown` without libstd
//! - Key-value state storage via [`state::set`] / [`state::get`]
//! - Structured event emission via [`event::emit`]
//! - Native CIL transfers via [`transfer`]
//! - Caller/contract context via [`caller`], [`self_address`], [`balance`]
//! - Blake3 hashing via [`crypto::blake3`]
//! - Custom global allocator for WASM heap
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! #![no_std]
//! #![no_main]
//! extern crate alloc;
//! extern crate los_sdk;
//!
//! use alloc::string::String;
//! use los_sdk::*;
//!
//! #[no_mangle]
//! pub extern "C" fn init() -> i32 {
//!     let name = arg(0).unwrap_or_default();
//!     state::set("name", name.as_bytes());
//!     event::emit("Init", "{}");
//!     0 // success
//! }
//! ```
//!
//! ## Compilation
//!
//! ```bash
//! cargo build --target wasm32-unknown-unknown --release
//! ```

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

#[cfg(target_arch = "wasm32")]
use core::fmt::Write as FmtWrite;

// ─────────────────────────────────────────────────────────────────
// WASM Global Allocator (growing bump allocator)
// ─────────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
mod allocator {
    use core::alloc::{GlobalAlloc, Layout};
    use core::sync::atomic::{AtomicUsize, Ordering};

    /// Bump allocator that grows WASM linear memory via `memory.grow`.
    /// Suitable for short-lived WASM executions (single function call).
    struct WasmBumpAllocator;

    /// Current allocation pointer (next free byte).
    static HEAP_PTR: AtomicUsize = AtomicUsize::new(0);
    /// End of current heap region.
    static HEAP_END: AtomicUsize = AtomicUsize::new(0);

    unsafe impl GlobalAlloc for WasmBumpAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let mut ptr = HEAP_PTR.load(Ordering::Relaxed);

            // First allocation: initialize heap
            if ptr == 0 {
                let page = core::arch::wasm32::memory_grow(0, 4); // Grow 4 pages (256KB)
                if page == usize::MAX {
                    return core::ptr::null_mut();
                }
                ptr = page * 65536;
                HEAP_END.store(ptr + 4 * 65536, Ordering::Relaxed);
            }

            // Align
            let align = layout.align();
            let aligned = (ptr + align - 1) & !(align - 1);
            let new_ptr = aligned + layout.size();

            let end = HEAP_END.load(Ordering::Relaxed);
            if new_ptr > end {
                // Grow more pages
                let needed_bytes = new_ptr - end;
                let needed_pages = (needed_bytes + 65535) / 65536;
                let page = core::arch::wasm32::memory_grow(0, needed_pages);
                if page == usize::MAX {
                    return core::ptr::null_mut();
                }
                HEAP_END.store(end + needed_pages * 65536, Ordering::Relaxed);
            }

            HEAP_PTR.store(new_ptr, Ordering::Relaxed);
            aligned as *mut u8
        }

        unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
            // Bump allocator: no individual deallocation.
            // Memory is reclaimed when the WASM instance is destroyed.
        }
    }

    #[global_allocator]
    static ALLOC: WasmBumpAllocator = WasmBumpAllocator;
}

// Provide a panic handler for `no_std` WASM builds
#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic_handler(info: &core::panic::PanicInfo) -> ! {
    // Try to extract a message and abort
    let mut buf = [0u8; 128];
    let mut cursor = WriteCursor::new(&mut buf);
    let _ = write!(cursor, "{}", info);
    let len = cursor.pos;
    unsafe {
        host_abort(buf.as_ptr(), len as u32);
        core::arch::wasm32::unreachable();
    }
}

/// Helper for writing panic messages into a fixed buffer.
#[cfg(target_arch = "wasm32")]
struct WriteCursor<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

#[cfg(target_arch = "wasm32")]
impl<'a> WriteCursor<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }
}

#[cfg(target_arch = "wasm32")]
impl<'a> FmtWrite for WriteCursor<'a> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let remaining = self.buf.len() - self.pos;
        let copy_len = bytes.len().min(remaining);
        self.buf[self.pos..self.pos + copy_len].copy_from_slice(&bytes[..copy_len]);
        self.pos += copy_len;
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────
// Host function imports (extern "C" from UVM)
// ─────────────────────────────────────────────────────────────────

extern "C" {
    fn host_log(ptr: *const u8, len: u32);
    fn host_abort(ptr: *const u8, len: u32);

    fn host_set_state(key_ptr: *const u8, key_len: u32, val_ptr: *const u8, val_len: u32);
    fn host_get_state(key_ptr: *const u8, key_len: u32, out_ptr: *mut u8, out_max: u32) -> i32;
    fn host_del_state(key_ptr: *const u8, key_len: u32);

    fn host_emit_event(type_ptr: *const u8, type_len: u32, data_ptr: *const u8, data_len: u32);

    fn host_transfer(addr_ptr: *const u8, addr_len: u32, amount_lo: i64, amount_hi: i64) -> i32;

    fn host_get_caller(out_ptr: *mut u8, out_max: u32) -> i32;
    fn host_get_self_address(out_ptr: *mut u8, out_max: u32) -> i32;
    fn host_get_balance_lo() -> i64;
    fn host_get_balance_hi() -> i64;
    fn host_get_timestamp() -> i64;

    fn host_get_arg_count() -> i32;
    fn host_get_arg(idx: i32, out_ptr: *mut u8, out_max: u32) -> i32;

    fn host_set_return(ptr: *const u8, len: u32);

    fn host_blake3(data_ptr: *const u8, data_len: u32, out_ptr: *mut u8) -> i32;
}

// ─────────────────────────────────────────────────────────────────
// Safe wrappers — State management
// ─────────────────────────────────────────────────────────────────

/// Contract state (persistent key-value storage).
pub mod state {
    use super::*;

    /// Write a key-value pair to the contract's persistent state.
    /// Both key and value are arbitrary bytes. Overwrites existing values.
    pub fn set(key: &str, value: &[u8]) {
        unsafe {
            host_set_state(
                key.as_ptr(),
                key.len() as u32,
                value.as_ptr(),
                value.len() as u32,
            );
        }
    }

    /// Write a UTF-8 string value to state.
    pub fn set_str(key: &str, value: &str) {
        set(key, value.as_bytes());
    }

    /// Write a u128 value to state (stored as 16-byte little-endian).
    pub fn set_u128(key: &str, value: u128) {
        let bytes = value.to_le_bytes();
        set(key, &bytes);
    }

    /// Write a u64 value to state (stored as 8-byte little-endian).
    pub fn set_u64(key: &str, value: u64) {
        let bytes = value.to_le_bytes();
        set(key, &bytes);
    }

    /// Read a value from the contract's state. Returns `None` if key not found.
    pub fn get(key: &str) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; 262_144]; // MAX_STATE_VALUE_SIZE
        let len = unsafe {
            host_get_state(
                key.as_ptr(),
                key.len() as u32,
                buf.as_mut_ptr(),
                buf.len() as u32,
            )
        };
        if len < 0 {
            return None;
        }
        buf.truncate(len as usize);
        Some(buf)
    }

    /// Read a UTF-8 string from state. Returns `None` if key not found or invalid UTF-8.
    pub fn get_str(key: &str) -> Option<String> {
        let bytes = get(key)?;
        String::from_utf8(bytes).ok()
    }

    /// Read a u128 value from state. Returns 0 if key not found or data too short.
    pub fn get_u128(key: &str) -> u128 {
        match get(key) {
            Some(bytes) if bytes.len() >= 16 => {
                let mut arr = [0u8; 16];
                arr.copy_from_slice(&bytes[..16]);
                u128::from_le_bytes(arr)
            }
            _ => 0,
        }
    }

    /// Read a u64 value from state. Returns 0 if key not found or data too short.
    pub fn get_u64(key: &str) -> u64 {
        match get(key) {
            Some(bytes) if bytes.len() >= 8 => {
                let mut arr = [0u8; 8];
                arr.copy_from_slice(&bytes[..8]);
                u64::from_le_bytes(arr)
            }
            _ => 0,
        }
    }

    /// Delete a key from the contract's state.
    pub fn del(key: &str) {
        unsafe {
            host_del_state(key.as_ptr(), key.len() as u32);
        }
    }

    /// Check if a key exists in state.
    pub fn exists(key: &str) -> bool {
        // Try reading 1 byte — if result >= 0, key exists
        let mut buf = [0u8; 1];
        let len = unsafe { host_get_state(key.as_ptr(), key.len() as u32, buf.as_mut_ptr(), 0) };
        len >= 0
    }
}

// ─────────────────────────────────────────────────────────────────
// Safe wrappers — Events
// ─────────────────────────────────────────────────────────────────

/// Structured event emission.
pub mod event {
    use super::*;

    /// Emit a structured event.
    /// - `event_type`: Short identifier (e.g., "Transfer", "Approval", "Swap")
    /// - `data_json`: JSON string with event data, e.g., `{"from":"LOSW...","amount":"1000"}`
    pub fn emit(event_type: &str, data_json: &str) {
        unsafe {
            host_emit_event(
                event_type.as_ptr(),
                event_type.len() as u32,
                data_json.as_ptr(),
                data_json.len() as u32,
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────
// Safe wrappers — Cryptography
// ─────────────────────────────────────────────────────────────────

/// Cryptographic utilities.
pub mod crypto {
    use super::*;

    /// Compute blake3 hash of `data`. Returns 32-byte hash.
    pub fn blake3(data: &[u8]) -> [u8; 32] {
        let mut out = [0u8; 32];
        unsafe {
            host_blake3(data.as_ptr(), data.len() as u32, out.as_mut_ptr());
        }
        out
    }
}

// ─────────────────────────────────────────────────────────────────
// Safe wrappers — Context
// ─────────────────────────────────────────────────────────────────

/// Get the caller's LOS address (verified from block signature by the node).
pub fn caller() -> String {
    let mut buf = [0u8; 256];
    let len = unsafe { host_get_caller(buf.as_mut_ptr(), buf.len() as u32) };
    if len <= 0 {
        return String::new();
    }
    String::from_utf8_lossy(&buf[..len as usize]).into_owned()
}

/// Get this contract's own address (LOSCon...).
pub fn self_address() -> String {
    let mut buf = [0u8; 256];
    let len = unsafe { host_get_self_address(buf.as_mut_ptr(), buf.len() as u32) };
    if len <= 0 {
        return String::new();
    }
    String::from_utf8_lossy(&buf[..len as usize]).into_owned()
}

/// Get the contract's current CIL balance (u128).
pub fn balance() -> u128 {
    let lo = unsafe { host_get_balance_lo() } as u64 as u128;
    let hi = unsafe { host_get_balance_hi() } as u64 as u128;
    (hi << 64) | lo
}

/// Get the current block timestamp (seconds since UNIX epoch).
pub fn timestamp() -> u64 {
    unsafe { host_get_timestamp() as u64 }
}

// ─────────────────────────────────────────────────────────────────
// Safe wrappers — Arguments
// ─────────────────────────────────────────────────────────────────

/// Get the number of arguments passed to this function call.
pub fn arg_count() -> u32 {
    let n = unsafe { host_get_arg_count() };
    if n < 0 {
        0
    } else {
        n as u32
    }
}

/// Get an argument by index. Returns `None` if index is out of bounds.
pub fn arg(idx: u32) -> Option<String> {
    let mut buf = vec![0u8; 65536]; // 64KB max arg size
    let len = unsafe { host_get_arg(idx as i32, buf.as_mut_ptr(), buf.len() as u32) };
    if len < 0 {
        return None;
    }
    buf.truncate(len as usize);
    String::from_utf8(buf).ok()
}

// ─────────────────────────────────────────────────────────────────
// Safe wrappers — Transfers
// ─────────────────────────────────────────────────────────────────

/// Transfer CIL from this contract to `recipient`.
/// Returns `Ok(())` on success, or an error message on failure.
pub fn transfer(recipient: &str, amount: u128) -> Result<(), &'static str> {
    let lo = (amount & 0xFFFF_FFFF_FFFF_FFFF) as u64 as i64;
    let hi = (amount >> 64) as u64 as i64;
    let result = unsafe { host_transfer(recipient.as_ptr(), recipient.len() as u32, lo, hi) };
    match result {
        0 => Ok(()),
        1 => Err("Insufficient contract balance"),
        2 => Err("Invalid recipient address"),
        3 => Err("Too many transfers in single execution"),
        _ => Err("Unknown transfer error"),
    }
}

// ─────────────────────────────────────────────────────────────────
// Safe wrappers — Return data
// ─────────────────────────────────────────────────────────────────

/// Set the return data for this contract call.
/// The caller (REST API or gossip handler) will receive this data.
pub fn set_return(data: &[u8]) {
    unsafe {
        host_set_return(data.as_ptr(), data.len() as u32);
    }
}

/// Set return data as a UTF-8 string.
pub fn set_return_str(s: &str) {
    set_return(s.as_bytes());
}

// ─────────────────────────────────────────────────────────────────
// Safe wrappers — Logging
// ─────────────────────────────────────────────────────────────────

/// Write a debug log line. Visible in node logs, not stored on-chain.
pub fn log(msg: &str) {
    unsafe {
        host_log(msg.as_ptr(), msg.len() as u32);
    }
}

// ─────────────────────────────────────────────────────────────────
// Safe wrappers — Abort
// ─────────────────────────────────────────────────────────────────

/// Abort contract execution with a message. All state changes are reverted.
/// This function never returns.
pub fn abort(msg: &str) -> ! {
    unsafe {
        host_abort(msg.as_ptr(), msg.len() as u32);
        #[cfg(target_arch = "wasm32")]
        core::arch::wasm32::unreachable();
        #[cfg(not(target_arch = "wasm32"))]
        core::hint::unreachable_unchecked();
    }
}

// ─────────────────────────────────────────────────────────────────
// Exported allocation functions (used by host to write into guest memory)
// ─────────────────────────────────────────────────────────────────

/// Allocate `size` bytes of memory and return the pointer.
/// Called by the host when it needs to write data into guest memory.
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn __los_alloc(size: u32) -> *mut u8 {
    let layout = core::alloc::Layout::from_size_align(size as usize, 1);
    match layout {
        Ok(layout) => unsafe { alloc::alloc::alloc(layout) },
        Err(_) => core::ptr::null_mut(),
    }
}

/// Deallocate memory at `ptr` with `size` bytes.
#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn __los_dealloc(ptr: *mut u8, size: u32) {
    if ptr.is_null() || size == 0 {
        return;
    }
    let layout = core::alloc::Layout::from_size_align(size as usize, 1);
    if let Ok(layout) = layout {
        unsafe {
            alloc::alloc::dealloc(ptr, layout);
        }
    }
}

// ─────────────────────────────────────────────────────────────────
// Tests (native only — host functions are mocked)
// ─────────────────────────────────────────────────────────────────

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    // Note: Host functions are not available on native targets.
    // These tests verify SDK type-level and logic correctness only.
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn test_u128_split_reconstruct() {
        let amount: u128 = 1_000_000_000_000; // 1 trillion
        let lo = (amount & 0xFFFF_FFFF_FFFF_FFFF) as u64;
        let hi = (amount >> 64) as u64;
        let reconstructed = ((hi as u128) << 64) | (lo as u128);
        assert_eq!(reconstructed, amount);
    }

    #[test]
    fn test_u128_max() {
        let amount: u128 = u128::MAX;
        let lo = (amount & 0xFFFF_FFFF_FFFF_FFFF) as u64;
        let hi = (amount >> 64) as u64;
        let reconstructed = ((hi as u128) << 64) | (lo as u128);
        assert_eq!(reconstructed, amount);
    }

    #[test]
    fn test_u128_le_bytes_roundtrip() {
        let val: u128 = 42_000_000_000;
        let bytes = val.to_le_bytes();
        let mut arr = [0u8; 16];
        arr.copy_from_slice(&bytes);
        let back = u128::from_le_bytes(arr);
        assert_eq!(back, val);
    }

    #[test]
    fn test_u64_le_bytes_roundtrip() {
        let val: u64 = 1700000000;
        let bytes = val.to_le_bytes();
        let mut arr = [0u8; 8];
        arr.copy_from_slice(&bytes);
        let back = u64::from_le_bytes(arr);
        assert_eq!(back, val);
    }

    // ── u128 split/reconstruct edge cases ──────────────────────

    #[test]
    fn test_u128_split_zero() {
        let amount: u128 = 0;
        let lo = (amount & 0xFFFF_FFFF_FFFF_FFFF) as u64;
        let hi = (amount >> 64) as u64;
        assert_eq!(lo, 0);
        assert_eq!(hi, 0);
        let reconstructed = ((hi as u128) << 64) | (lo as u128);
        assert_eq!(reconstructed, 0);
    }

    #[test]
    fn test_u128_split_only_lo() {
        // Value fits entirely in lower 64 bits
        let amount: u128 = u64::MAX as u128;
        let lo = (amount & 0xFFFF_FFFF_FFFF_FFFF) as u64;
        let hi = (amount >> 64) as u64;
        assert_eq!(lo, u64::MAX);
        assert_eq!(hi, 0);
    }

    #[test]
    fn test_u128_split_only_hi() {
        // Value has only upper 64 bits set
        let amount: u128 = (1u128) << 64;
        let lo = (amount & 0xFFFF_FFFF_FFFF_FFFF) as u64;
        let hi = (amount >> 64) as u64;
        assert_eq!(lo, 0);
        assert_eq!(hi, 1);
        let reconstructed = ((hi as u128) << 64) | (lo as u128);
        assert_eq!(reconstructed, amount);
    }

    #[test]
    fn test_u128_split_los_total_supply() {
        // Total supply: 21,936,236 LOS × 10^11 = 2,193,623,600,000,000,000 CIL
        let total_supply_cil: u128 = 2_193_623_600_000_000_000;
        let lo = (total_supply_cil & 0xFFFF_FFFF_FFFF_FFFF) as u64;
        let hi = (total_supply_cil >> 64) as u64;
        // Total supply fits in 64 bits (< 2^64)
        assert_eq!(hi, 0);
        assert_eq!(lo, 2_193_623_600_000_000_000);
        let reconstructed = ((hi as u128) << 64) | (lo as u128);
        assert_eq!(reconstructed, total_supply_cil);
    }

    // ── Transfer amount encoding ────────────────────────────────

    #[test]
    fn test_transfer_amount_i64_encoding() {
        // Verify the i64 cast pattern used in transfer()
        let amount: u128 = 100_000_000_000; // 1 LOS in CIL
        let lo = (amount & 0xFFFF_FFFF_FFFF_FFFF) as u64 as i64;
        let hi = (amount >> 64) as u64 as i64;
        // Reconstruct
        let back_lo = lo as u64 as u128;
        let back_hi = hi as u64 as u128;
        assert_eq!((back_hi << 64) | back_lo, amount);
    }

    #[test]
    fn test_transfer_amount_large_i64_encoding() {
        // Test with amount that requires both lo and hi parts
        let amount: u128 = u128::MAX;
        let lo = (amount & 0xFFFF_FFFF_FFFF_FFFF) as u64 as i64;
        let hi = (amount >> 64) as u64 as i64;
        // Reconstruct through the same cast chain
        let back_lo = lo as u64 as u128;
        let back_hi = hi as u64 as u128;
        assert_eq!((back_hi << 64) | back_lo, amount);
    }

    // ── LE bytes roundtrips for state storage ───────────────────

    #[test]
    fn test_u128_le_bytes_zero() {
        let val: u128 = 0;
        let bytes = val.to_le_bytes();
        assert_eq!(u128::from_le_bytes(bytes), 0);
    }

    #[test]
    fn test_u128_le_bytes_max() {
        let val: u128 = u128::MAX;
        let bytes = val.to_le_bytes();
        assert_eq!(u128::from_le_bytes(bytes), u128::MAX);
    }

    #[test]
    fn test_u64_le_bytes_max() {
        let val: u64 = u64::MAX;
        let bytes = val.to_le_bytes();
        assert_eq!(u64::from_le_bytes(bytes), u64::MAX);
    }

    #[test]
    fn test_u128_le_bytes_partial_decode() {
        // Simulate state::get_u128 when data is too short (should return 0)
        let short_bytes: Vec<u8> = vec![1, 2, 3]; // < 16 bytes
        let result = if short_bytes.len() >= 16 {
            let mut arr = [0u8; 16];
            arr.copy_from_slice(&short_bytes[..16]);
            u128::from_le_bytes(arr)
        } else {
            0 // Fallback per SDK contract
        };
        assert_eq!(result, 0);
    }

    #[test]
    fn test_u64_le_bytes_partial_decode() {
        // Simulate state::get_u64 when data is too short
        let short_bytes: Vec<u8> = vec![1, 2]; // < 8 bytes
        let result = if short_bytes.len() >= 8 {
            let mut arr = [0u8; 8];
            arr.copy_from_slice(&short_bytes[..8]);
            u64::from_le_bytes(arr)
        } else {
            0
        };
        assert_eq!(result, 0);
    }

    // ── Layout validation (alloc/dealloc) ───────────────────────

    #[test]
    fn test_layout_from_size_align() {
        // Verify the Layout creation pattern used in __los_alloc
        let layout = core::alloc::Layout::from_size_align(1024, 1);
        assert!(layout.is_ok());
        let layout = layout.unwrap();
        assert_eq!(layout.size(), 1024);
        assert_eq!(layout.align(), 1);
    }

    #[test]
    fn test_layout_zero_size() {
        let layout = core::alloc::Layout::from_size_align(0, 1);
        assert!(layout.is_ok()); // Zero-size layouts are valid
    }

    #[test]
    fn test_layout_invalid_alignment() {
        // Alignment must be a power of two
        let layout = core::alloc::Layout::from_size_align(1024, 3);
        assert!(layout.is_err());
    }
}
