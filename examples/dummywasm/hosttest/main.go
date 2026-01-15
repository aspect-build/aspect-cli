// Demo wasm module that tests host function imports.
//
// Build with: cd hosttest && GOOS=wasip1 GOARCH=wasm go build -o ../host_test.wasm .
package main

// Host function imports - these will be provided by AXL
//
//go:wasmimport env get_magic_number
func get_magic_number() int32

//go:wasmimport env add_numbers
func add_numbers(a int32, b int32) int32

// Exported function that calls the host function
//
//go:wasmexport call_get_magic
func call_get_magic() int32 {
	return get_magic_number()
}

// Exported function that calls add_numbers host function
//
//go:wasmexport call_add
func call_add(a int32, b int32) int32 {
	return add_numbers(a, b)
}

// Exported function that combines both host functions
//
//go:wasmexport get_magic_plus_sum
func get_magic_plus_sum(a int32, b int32) int32 {
	magic := get_magic_number()
	sum := add_numbers(a, b)
	return magic + sum
}

func main() {}
