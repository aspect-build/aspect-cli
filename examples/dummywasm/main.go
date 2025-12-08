// Demo wasm module that returns dummy JSON data via memory.
//
// Build with: GOOS=wasip1 GOARCH=wasm go build -o dummy.wasm .
package main

import (
	"unsafe"
)

// Dummy JSON data to be written to memory
var dummyJSON = `{"name":"dummy","version":"1.0.0","enabled":true,"count":42}`

// get_dummy_json allocates memory for the JSON data and returns both
// the pointer and length packed into a single uint64:
//   - lower 32 bits: memory pointer to the JSON data
//   - upper 32 bits: length of the JSON data
//
// We pack two values because Go's wasmexport doesn't support multiple
// return values. The caller unpacks with:
//
//	ptr = result & 0xFFFFFFFF
//	length = (result >> 32) & 0xFFFFFFFF
//
//go:wasmexport get_dummy_json
func get_dummy_json() uint64 {
	data := []byte(dummyJSON)
	buf := make([]byte, len(data))
	copy(buf, data)
	ptr := uint32(uintptr(unsafe.Pointer(&buf[0])))
	length := uint32(len(data))
	return uint64(ptr) | (uint64(length) << 32)
}

func main() {}
