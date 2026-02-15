// Simple WASI-enabled wasm module for exercising the AXL wasm engine.
//
// Build with:
//
//	GOOS=wasip1 GOARCH=wasm go build -o imagination.wasm .
package main

import "unsafe"

// host_add is provided by the host via WASM imports.
//
//go:wasmimport env host_add
func host_add(a int32, b int32) int32

// message is returned to the host via a pointer/length pair.
var message = []byte("Hello from the imagination wasm module!")

// get_message returns a packed pointer/length pair (lower 32 bits pointer, upper 32 bits length).
//
//go:wasmexport get_message
func get_message() uint64 {
	ptr := uint32(uintptr(unsafe.Pointer(&message[0])))
	length := uint32(len(message))
	return uint64(ptr) | (uint64(length) << 32)
}

// add_numbers adds two integers inside the wasm module.
//
//go:wasmexport add_numbers
func add_numbers(a int32, b int32) int32 {
	return a + b
}

// add_with_host forwards to the host-provided add function.
//
//go:wasmexport add_with_host
func add_with_host(a int32, b int32) int32 {
	return host_add(a, b)
}

func main() {}
