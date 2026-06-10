# RC++ (Reviewed C++)

RC++ (Reviewed C++) is a strictly reviewed and refined dialect of C++ designed specifically for systems programming and OS kernel development. It eliminates preprocessor magic, hidden allocations, and unpredictable behaviors, providing a clean, explicit, and robust syntax for building reliable systems.

`rcpp` is a transpiler and build driver for RC++ that translates `.rcx` files into standard, highly-optimized C++26 and compiles them via Clang or GCC.

## Philosophy

RC++ is not a new language or a hybrid. It is C++ that has been **reviewed** to keep only what is safe, explicit, and zero-cost, while removing the footguns that make traditional C++ difficult for kernel development.

## Features

### Explicit and Clean Syntax

```cpp
// Explicit type inference (no hidden macros)
let x = 42;           // transpiles to: const auto x = 42;
mut counter = 0;      // transpiles to: auto counter = 0;

// Clean function declarations with explicit return types
fn add(a: i32, b: i32) -> i32 {
    return a + b;
}
// transpiles to: int32_t add(int32_t a, int32_t b) { return a + b; }

// Safe, context-rich error handling without exceptions
fn divide(a: i32, b: i32) -> i32 {
    if b == 0 {
        panic!("Division by zero");
    }
    return a / b;
}
// transpiles to: __panic_here(__FILE__, __LINE__, "module_name", "Division by zero");
```

### Strict Module System

```cpp
// Clear separation of interface and implementation
interface {
    struct Person {
        u8* _name;
        u8  _age;
        fn age() -> u8;
    }
}

fn Person::age() -> u8 {
    return _age;
}

// Explicit, predictable imports
use person::*;

// External module dependencies
extern mod my_lib "/path/to/lib";
```

### Declarative Attributes

```cpp
// Conditional compilation without preprocessor spaghetti
#[cfg(arch = "x86_64")]
fn init_gdt() { /* ... */ }

// Explicit function attributes
#[no_mangle]
#[export_name = "kernel_entry"]
fn _start() { /* ... */ }
```

## Usage

### Basic compilation

```bash
rcpp $project_dir -r $root_module --invoke-cc clang++ -- -O3 -Wall
```

### With external modules

```bash
rcpp $project_dir \
    -r my_kernel \
    -e my_lib,/path/to/lib \
    --invoke-cc clang++ -- -O3 -Wall
```

### JSON-RPC mode (for IDE / custom build system integration)

```bash
rcpp $project_dir --json-rpc
```

Send JSON-RPC requests via stdin:
```json
{
    "jsonrpc": "2.0",
    "method": "build",
    "id": 1,
    "params": {
        "project_dir": "example/",
        "root_module": "my_kernel",
        "cc_path": "clang++",
        "cc_args": ["-O3", "-Wall"]
    }
}
```

## Project Structure

```text
example/
|-- Fuser.toml
\-- src/
    |-- _.rcx          # Root module (becomes <root_module>.cpp/.hpp)
    \-- person.rcx     # Submodule (becomes <root_module>::person)
```

## License

Licensed under either of:
- Apache License, Version 2.0 (LICENSE-APACHE)
- MIT License (LICENSE-MIT)

at your option.
