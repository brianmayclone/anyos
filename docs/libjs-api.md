# anyOS JavaScript Engine (libjs) API Reference

The **libjs** crate is a complete `#![no_std]` ECMAScript-compatible JavaScript engine for anyOS. It includes a lexer/tokenizer, recursive descent parser (ES2020+ syntax), AST representation, bytecode compiler, and a stack-based virtual machine with prototype chains, closures, and async/await support.

**Crate:** `libjs` (version 0.1.0, edition 2021)
**Location:** `libs/libjs/`
**Type:** Rust library (statically linked, not a DLL)

---

## Table of Contents

- [Getting Started](#getting-started)
- [JsEngine -- High-Level API](#jsengine----high-level-api)
- [JsValue -- Value Types](#jsvalue----value-types)
  - [Constructors](#constructors)
  - [Type Checks](#type-checks)
  - [Type Conversions](#type-conversions)
  - [Equality](#equality)
  - [Property Access](#property-access)
- [JsObject](#jsobject)
- [JsArray](#jsarray)
- [JsFunction](#jsfunction)
- [Property Descriptors](#property-descriptors)
- [Vm -- Virtual Machine](#vm----virtual-machine)
- [Bytecode -- Chunk and Op](#bytecode----chunk-and-op)
- [Lexer, Parser, Compiler](#lexer-parser-compiler)
- [Built-in Globals](#built-in-globals)
  - [Global Functions](#global-functions)
  - [Constructors](#constructors-1)
  - [console](#console)
  - [Math](#math)
  - [JSON](#json)
  - [Object Static Methods](#object-static-methods)
  - [Array Static Methods](#array-static-methods)
  - [Promise](#promise)
  - [Map and Set](#map-and-set)
  - [Date](#date)
  - [Timers](#timers)
  - [Symbol](#symbol)
  - [Proxy](#proxy)
- [Built-in Prototype Methods](#built-in-prototype-methods)
  - [Array.prototype](#arrayprototype)
  - [String.prototype](#stringprototype)
  - [Number.prototype](#numberprototype)
  - [Function.prototype](#functionprototype)
  - [Error.prototype](#errorprototype)
- [Bytecode Instruction Reference](#bytecode-instruction-reference)
- [Native Function Signature](#native-function-signature)
- [Examples](#examples)

---

## Getting Started

### Dependencies

Add to your crate's `Cargo.toml`:

```toml
[dependencies]
libjs = { path = "../../libs/libjs" }
```

libjs depends on `anyos_std` internally but your crate does not need to depend on it unless you use stdlib features directly.

### Minimal Example

```rust
use libjs::JsEngine;

let mut engine = JsEngine::new();

// Evaluate expressions
let result = engine.eval("2 + 3 * 4");
assert_eq!(result.to_number(), 14.0);

// Run statements
engine.eval("var greeting = 'Hello, ' + 'world!';");
let val = engine.get_global("greeting");
assert_eq!(val.to_js_string(), "Hello, world!");

// Console output is captured
engine.eval("console.log('test message');");
for msg in engine.console_output() {
    // msg == "test message"
}
```

---

## JsEngine -- High-Level API

`JsEngine` is the primary interface. It wraps the VM and provides a simple evaluate-and-inspect workflow.

```rust
pub struct JsEngine { /* private */ }
```

### Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `() -> JsEngine` | Create a new engine with all built-ins initialized |
| `eval` | `(&mut self, source: &str) -> JsValue` | Parse, compile, and execute JavaScript source; returns the result |
| `set_global` | `(&mut self, name: &str, value: JsValue)` | Set a global variable |
| `get_global` | `(&mut self, name: &str) -> JsValue` | Get a global variable (returns `Undefined` if not found) |
| `register_native` | `(&mut self, name: &str, func: fn(&mut Vm, &[JsValue]) -> JsValue)` | Register a Rust function as a global JS function |
| `console_output` | `(&self) -> &[String]` | Get all `console.log`/`warn`/`error` output messages |
| `clear_console` | `(&mut self)` | Clear accumulated console output |
| `set_step_limit` | `(&mut self, limit: u64)` | Set max execution steps (default: 10,000,000); prevents infinite loops |
| `vm` | `(&mut self) -> &mut Vm` | Access the underlying VM directly |
| `compile` | `(&self, source: &str) -> Chunk` | Compile JS source to bytecode without executing it |

### eval Pipeline

When you call `engine.eval(source)`, the engine runs four stages:

1. **Lexer** -- Tokenizes the source string into a `Vec<Token>`
2. **Parser** -- Builds an AST (`Program`) from the token stream
3. **Compiler** -- Compiles the AST into a `Chunk` (bytecode + constant pool)
4. **VM** -- Executes the chunk and returns the final value

---

## JsValue -- Value Types

All JavaScript values are represented by the `JsValue` enum. Objects, Arrays, and Functions use `Rc<RefCell<>>` for reference semantics -- cloning a `JsValue` only bumps the reference count, so mutations through one handle are visible through all others.

```rust
pub enum JsValue {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Object(Rc<RefCell<JsObject>>),
    Array(Rc<RefCell<JsArray>>),
    Function(Rc<RefCell<JsFunction>>),
}
```

### Constructors

| Function | Signature | Description |
|----------|-----------|-------------|
| `JsValue::new_object` | `() -> JsValue` | Create a new empty JS object |
| `JsValue::new_array` | `(Vec<JsValue>) -> JsValue` | Create a new array from elements |
| `JsValue::new_function` | `(JsFunction) -> JsValue` | Wrap a `JsFunction` in `Rc<RefCell>` |

### Type Checks

All return `bool`:

| Method | True when |
|--------|-----------|
| `is_undefined()` | Value is `Undefined` |
| `is_null()` | Value is `Null` |
| `is_nullish()` | Value is `Undefined` or `Null` |
| `is_number()` | Value is `Number` |
| `is_string()` | Value is `String` |
| `is_bool()` | Value is `Bool` |
| `is_object()` | Value is `Object` |
| `is_array()` | Value is `Array` |
| `is_function()` | Value is `Function` |

### Type Conversions

These follow ECMAScript abstract operation semantics:

| Method | Signature | Description |
|--------|-----------|-------------|
| `to_boolean` | `(&self) -> bool` | ToBoolean: `0`, `NaN`, `""`, `null`, `undefined` are false; objects/arrays/functions always true |
| `to_number` | `(&self) -> f64` | ToNumber: `undefined`->NaN, `null`->0, `true`->1, strings parsed as float |
| `to_js_string` | `(&self) -> String` | ToString: `[object Object]` for objects, comma-joined for arrays |
| `type_of` | `(&self) -> &'static str` | typeof operator result: `"undefined"`, `"boolean"`, `"number"`, `"string"`, `"object"`, `"function"` |

Note: `type_of()` returns `"object"` for `Null` (matching the historical JS behavior).

### Equality

| Method | Signature | Description |
|--------|-----------|-------------|
| `abstract_eq` | `(&self, other: &JsValue) -> bool` | Abstract equality (`==`): coerces types, `null == undefined` is true |
| `strict_eq` | `(&self, other: &JsValue) -> bool` | Strict equality (`===`): no coercion, objects compared by reference identity |

### Property Access

| Method | Signature | Description |
|--------|-----------|-------------|
| `get_property` | `(&self, key: &str) -> JsValue` | Get property on objects, arrays, or strings (includes `length`) |
| `set_property` | `(&self, key: String, value: JsValue)` | Set property (objects, arrays, functions); silently ignored on primitives |
| `delete_property` | `(&self, key: &str) -> bool` | Delete a property from an object |

Property access on arrays supports numeric index strings and `"length"`. Property access on strings supports `"length"` and character indexing.

---

## JsObject

A JavaScript object (property map with prototype chain).

```rust
pub struct JsObject {
    pub properties: BTreeMap<String, Property>,
    pub prototype: Option<Rc<RefCell<JsObject>>>,
    pub internal_tag: Option<String>,
    pub set_hook: Option<fn(*mut u8, &str, &JsValue)>,
    pub set_hook_data: *mut u8,
}
```

### Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `() -> JsObject` | Create an empty object with no prototype |
| `with_tag` | `(tag: &str) -> JsObject` | Create an object with an internal tag (e.g., `"Map"`, `"Set"`) |
| `get` | `(&self, key: &str) -> JsValue` | Get property value, walking the prototype chain |
| `set` | `(&mut self, key: String, value: JsValue)` | Set a property (triggers set_hook if installed) |
| `set_hidden` | `(&mut self, key: String, value: JsValue)` | Set a non-enumerable property |
| `has` | `(&self, key: &str) -> bool` | Check if property exists (walks prototype chain) |
| `has_own` | `(&self, key: &str) -> bool` | Check own properties only (no prototype walk) |
| `delete` | `(&mut self, key: &str) -> bool` | Delete a configurable property |
| `keys` | `(&self) -> Vec<String>` | Get all enumerable own property names |

The `set_hook` field allows host code to be notified when a property is set. This is used internally by built-in types (e.g., Proxy traps).

---

## JsArray

A JavaScript array (indexed elements plus named properties).

```rust
pub struct JsArray {
    pub elements: Vec<JsValue>,
    pub properties: BTreeMap<String, Property>,
}
```

### Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `() -> JsArray` | Create an empty array |
| `from_vec` | `(Vec<JsValue>) -> JsArray` | Create an array from existing elements |
| `get` | `(&self, index: usize) -> JsValue` | Get element by index (returns `Undefined` if out of bounds) |
| `set` | `(&mut self, index: usize, value: JsValue)` | Set element (auto-extends with `Undefined` if needed) |
| `push` | `(&mut self, value: JsValue)` | Append an element |
| `len` | `(&self) -> usize` | Get element count |

---

## JsFunction

A compiled or native JavaScript function.

```rust
pub struct JsFunction {
    pub name: Option<String>,
    pub params: Vec<String>,
    pub kind: FnKind,
    pub this_binding: Option<JsValue>,
    pub upvalues: Vec<Rc<RefCell<JsValue>>>,
    pub prototype: Option<Rc<RefCell<JsObject>>>,
    pub own_props: BTreeMap<String, JsValue>,
}
```

### FnKind

```rust
pub enum FnKind {
    Bytecode(Chunk),
    Native(fn(&mut Vm, &[JsValue]) -> JsValue),
}
```

- **Bytecode** -- A compiled JS function; the `Chunk` contains the bytecode, constants, and upvalue descriptors.
- **Native** -- A Rust function pointer that receives the VM and arguments.

The `this_binding` field is set when a function is called with `.bind()`. The `upvalues` field holds captured closure variables as shared `Rc<RefCell<JsValue>>` cells. The `prototype` field is the function's `.prototype` object used with `new`.

---

## Property Descriptors

```rust
pub struct Property {
    pub value: JsValue,
    pub writable: bool,
    pub enumerable: bool,
    pub configurable: bool,
}
```

### Factory Methods

| Method | writable | enumerable | configurable | Use case |
|--------|----------|------------|--------------|----------|
| `Property::data(value)` | true | true | true | Normal properties |
| `Property::readonly(value)` | false | true | false | Constants like `Math.PI` |
| `Property::hidden(value)` | true | false | true | Internal/non-enumerable properties |

---

## Vm -- Virtual Machine

The `Vm` is the low-level execution engine. You can use it directly for fine-grained control, or access it through `JsEngine::vm()`.

```rust
pub struct Vm {
    pub stack: Vec<JsValue>,
    pub frames: Vec<CallFrame>,
    pub globals: JsObject,
    pub try_handlers: Vec<TryHandler>,
    pub console_output: Vec<String>,
    pub engine_log: Vec<String>,
    pub object_proto: Rc<RefCell<JsObject>>,
    pub array_proto: Rc<RefCell<JsObject>>,
    pub string_proto: Rc<RefCell<JsObject>>,
    pub function_proto: Rc<RefCell<JsObject>>,
    pub number_proto: Rc<RefCell<JsObject>>,
    pub error_proto: Rc<RefCell<JsObject>>,
    pub step_limit: u64,
    pub steps: u64,
    pub userdata: *mut u8,
    pub current_this: JsValue,
    pub run_target_depth: usize,
}
```

### Key Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `() -> Vm` | Create a VM with all prototypes and globals initialized |
| `execute` | `(&mut self, chunk: Chunk) -> JsValue` | Execute a compiled chunk; resets step counter |
| `set_global` | `(&mut self, name: &str, value: JsValue)` | Set a global variable |
| `get_global` | `(&mut self, name: &str) -> JsValue` | Get a global variable |
| `register_native` | `(&mut self, name: &str, func: fn(&mut Vm, &[JsValue]) -> JsValue)` | Register a native function as a global |
| `set_step_limit` | `(&mut self, limit: u64)` | Set maximum execution steps |
| `run` | `(&mut self) -> JsValue` | Resume execution of current call frames |
| `log_engine` | `(&mut self, msg: &str)` | Append a diagnostic message to `engine_log` |

### Notable Fields

- **`console_output`** -- Accumulated `console.log`/`warn`/`error` messages. Inspect after `eval()` to capture JS console output.
- **`engine_log`** -- Internal diagnostic messages from the VM (initialization, errors, warnings).
- **`userdata`** -- Raw pointer for host-provided context; can be set before calling native functions that need external state.
- **`step_limit`** / **`steps`** -- Safety mechanism to abort runaway scripts. Default limit is 10,000,000 steps.
- **`*_proto`** fields -- Built-in prototype objects. You can extend these with additional methods.

### Helper Functions (module-level)

| Function | Signature | Description |
|----------|-----------|-------------|
| `vm::native_fn` | `(name: &str, f: fn(&mut Vm, &[JsValue]) -> JsValue) -> JsValue` | Create a `JsValue::Function` wrapping a native Rust function |
| `vm::get_proto_prop_rc` | `(proto: &Rc<RefCell<JsObject>>, key: &str) -> JsValue` | Walk a prototype chain to find a property |

---

## Bytecode -- Chunk and Op

### Chunk

A compiled code block containing bytecode instructions and a constant pool.

```rust
pub struct Chunk {
    pub code: Vec<Op>,
    pub constants: Vec<Constant>,
    pub local_count: u16,
    pub param_count: u16,
    pub name: Option<String>,
    pub upvalues: Vec<UpvalueRef>,
}
```

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `() -> Chunk` | Create an empty chunk |
| `add_const` | `(&mut self, c: Constant) -> u16` | Add a constant (deduplicates); returns index |
| `emit` | `(&mut self, op: Op) -> usize` | Emit an instruction; returns its index |
| `offset` | `(&self) -> usize` | Current code length (next instruction index) |
| `patch_jump` | `(&mut self, idx: usize)` | Patch a jump at `idx` to target current offset |

### Constant

```rust
pub enum Constant {
    Number(f64),
    String(String),
    Function(Chunk),   // nested function prototype
}
```

### UpvalueRef

Describes how a closure captures a variable from its enclosing scope:

```rust
pub struct UpvalueRef {
    pub is_local: bool,  // true = capture from enclosing locals; false = from enclosing upvalues
    pub index: u16,      // slot index in the enclosing scope
}
```

---

## Lexer, Parser, Compiler

These modules are public and can be used independently for custom pipelines.

### Lexer

```rust
use libjs::lexer::Lexer;

let tokens = Lexer::tokenize("var x = 42;");
```

### Parser

```rust
use libjs::parser::Parser;

let mut parser = Parser::new(tokens);
let program = parser.parse_program();  // -> Program (AST)
```

### Compiler

```rust
use libjs::compiler::Compiler;

let mut compiler = Compiler::new();
let chunk = compiler.compile(&program);  // -> Chunk (bytecode)
```

The AST types are defined in `libjs::ast` (`Program`, `Stmt`, `Expr`, etc.).

---

## Built-in Globals

These are automatically available after creating a `JsEngine` or `Vm`.

### Global Functions

| Function | Description |
|----------|-------------|
| `parseInt(string, radix?)` | Parse an integer from a string |
| `parseFloat(string)` | Parse a floating-point number |
| `isNaN(value)` | Check if value is NaN |
| `isFinite(value)` | Check if value is finite |
| `encodeURIComponent(str)` | Percent-encode a URI component |
| `decodeURIComponent(str)` | Decode a percent-encoded URI component |

### Global Constants

| Name | Value |
|------|-------|
| `Infinity` | `f64::INFINITY` |
| `NaN` | `f64::NAN` |
| `undefined` | `JsValue::Undefined` |

### Constructors

| Constructor | Description |
|-------------|-------------|
| `Object()` | Object constructor |
| `Array()` | Array constructor |
| `String()` | String constructor/converter |
| `Number()` | Number constructor/converter |
| `Boolean()` | Boolean constructor/converter |
| `Error(message)` | Error constructor |
| `TypeError(message)` | TypeError constructor |
| `RangeError(message)` | RangeError constructor |
| `ReferenceError(message)` | ReferenceError constructor |
| `SyntaxError(message)` | SyntaxError constructor |

### console

| Method | Description |
|--------|-------------|
| `console.log(...)` | Log message (captured in `console_output`) |
| `console.warn(...)` | Log warning |
| `console.error(...)` | Log error |
| `console.info(...)` | Alias for `log` |
| `console.debug(...)` | Alias for `log` |

### Math

**Constants:** `PI`, `E`, `LN2`, `LN10`, `LOG2E`, `LOG10E`, `SQRT2`, `SQRT1_2`

**Methods:**

| Method | Description |
|--------|-------------|
| `Math.abs(x)` | Absolute value |
| `Math.floor(x)` | Round down |
| `Math.ceil(x)` | Round up |
| `Math.round(x)` | Round to nearest integer |
| `Math.trunc(x)` | Truncate to integer |
| `Math.max(...args)` | Maximum of arguments |
| `Math.min(...args)` | Minimum of arguments |
| `Math.pow(base, exp)` | Exponentiation |
| `Math.sqrt(x)` | Square root |
| `Math.cbrt(x)` | Cube root |
| `Math.sign(x)` | Sign of number (-1, 0, 1) |
| `Math.log(x)` | Natural logarithm |
| `Math.log2(x)` | Base-2 logarithm |
| `Math.log10(x)` | Base-10 logarithm |
| `Math.sin(x)` | Sine |
| `Math.cos(x)` | Cosine |
| `Math.tan(x)` | Tangent |
| `Math.atan2(y, x)` | Two-argument arctangent |
| `Math.hypot(...args)` | Euclidean distance |
| `Math.clz32(x)` | Count leading zeros (32-bit) |
| `Math.fround(x)` | Round to 32-bit float |
| `Math.random()` | Random number in [0, 1) |

### JSON

| Method | Description |
|--------|-------------|
| `JSON.parse(text)` | Parse a JSON string into a JS value |
| `JSON.stringify(value)` | Serialize a JS value to a JSON string |

### Object Static Methods

| Method | Description |
|--------|-------------|
| `Object.keys(obj)` | Array of own enumerable property names |
| `Object.values(obj)` | Array of own enumerable property values |
| `Object.entries(obj)` | Array of `[key, value]` pairs |
| `Object.assign(target, ...sources)` | Copy properties into target |
| `Object.freeze(obj)` | Make all properties non-writable and non-configurable |
| `Object.create(proto)` | Create object with specified prototype |
| `Object.defineProperty(obj, prop, desc)` | Define a property with descriptor |
| `Object.getPrototypeOf(obj)` | Get an object's prototype |

### Array Static Methods

| Method | Description |
|--------|-------------|
| `Array.isArray(value)` | Check if value is an array |
| `Array.from(iterable)` | Create array from iterable |
| `Array.of(...items)` | Create array from arguments |

### Promise

**Constructor:** `new Promise(executor)` where executor receives `(resolve, reject)`.

**Static methods:**

| Method | Description |
|--------|-------------|
| `Promise.resolve(value)` | Create a fulfilled promise |
| `Promise.reject(reason)` | Create a rejected promise |
| `Promise.all(iterable)` | Wait for all promises |
| `Promise.allSettled(iterable)` | Wait for all to settle |
| `Promise.race(iterable)` | First to settle wins |

Note: Since there is no event loop, `await` on a pending promise returns `undefined`. Fulfilled and rejected promises resolve immediately.

### Map and Set

| Constructor | Description |
|-------------|-------------|
| `new Map()` | Create a new Map |
| `new Set()` | Create a new Set |

Map and Set instances use internal tagged objects with standard methods (`get`, `set`, `has`, `delete`, `clear`, `size`, `forEach`, `keys`, `values`, `entries`).

### Date

| Constructor | Description |
|-------------|-------------|
| `new Date()` | Create a Date for current time |
| `new Date(milliseconds)` | Create a Date from timestamp |

**Static methods:**

| Method | Description |
|--------|-------------|
| `Date.now()` | Current timestamp in milliseconds |
| `Date.parse(string)` | Parse a date string |

### Timers

| Function | Description |
|----------|-------------|
| `setTimeout(fn, ms)` | Schedule a function (executed immediately in the VM) |
| `setInterval(fn, ms)` | Schedule a repeating function |
| `clearTimeout(id)` | Cancel a timeout |
| `clearInterval(id)` | Cancel an interval |

Note: Without an event loop, timers execute their callback once immediately upon creation.

### Symbol

| Usage | Description |
|-------|-------------|
| `Symbol(description?)` | Create a unique symbol |
| `Symbol.iterator` | Well-known iterator symbol |
| `Symbol.toPrimitive` | Well-known toPrimitive symbol |
| `Symbol.toStringTag` | Well-known toStringTag symbol |
| `Symbol.hasInstance` | Well-known hasInstance symbol |

### Proxy

| Usage | Description |
|-------|-------------|
| `new Proxy(target, handler)` | Create a proxy object with traps |
| `Proxy.revocable(target, handler)` | Create a revocable proxy |

---

## Built-in Prototype Methods

### Array.prototype

`push`, `pop`, `shift`, `unshift`, `indexOf`, `lastIndexOf`, `includes`, `join`, `slice`, `splice`, `concat`, `reverse`, `sort`, `map`, `filter`, `forEach`, `reduce`, `reduceRight`, `find`, `findIndex`, `some`, `every`, `flat`, `flatMap`, `fill`, `copyWithin`, `entries`, `keys`, `values`, `at`, `toString`

### String.prototype

`charAt`, `charCodeAt`, `codePointAt`, `indexOf`, `lastIndexOf`, `includes`, `startsWith`, `endsWith`, `slice`, `substring`, `toLowerCase`, `toUpperCase`, `trim`, `trimStart`, `trimEnd`, `split`, `replace`, `replaceAll`, `repeat`, `padStart`, `padEnd`, `at`, `concat`, `toString`, `valueOf`

### Number.prototype

`toString`, `valueOf`, `toFixed`

### Function.prototype

`call`, `apply`, `bind`, `toString`

### Error.prototype

Properties: `name` (default `"Error"`), `message` (default `""`)
Methods: `toString`

---

## Bytecode Instruction Reference

The `Op` enum defines all bytecode instructions. They are grouped by category:

### Stack Operations

| Op | Description |
|----|-------------|
| `LoadConst(u16)` | Push constant from pool by index |
| `LoadUndefined` | Push `undefined` |
| `LoadNull` | Push `null` |
| `LoadTrue` | Push `true` |
| `LoadFalse` | Push `false` |
| `Pop` | Discard top value |
| `Dup` | Duplicate top value |

### Variable Operations

| Op | Description |
|----|-------------|
| `LoadLocal(u16)` | Load local variable by slot |
| `StoreLocal(u16)` | Store into local slot (keeps value on stack) |
| `LoadGlobal(u16)` | Load global by name (constant pool index) |
| `StoreGlobal(u16)` | Store into global |
| `LoadUpvalue(u16)` | Load closed-over variable |
| `StoreUpvalue(u16)` | Store into closed-over variable |

### Arithmetic and Bitwise

| Op | Description |
|----|-------------|
| `Add` | Addition (or string concatenation) |
| `Sub`, `Mul`, `Div`, `Mod`, `Exp` | Arithmetic operators |
| `Neg`, `Pos` | Unary minus, unary plus |
| `BitAnd`, `BitOr`, `BitXor`, `BitNot` | Bitwise operators |
| `Shl`, `Shr`, `UShr` | Shift operators |
| `Inc`, `Dec` | Increment, decrement |

### Comparison and Logical

| Op | Description |
|----|-------------|
| `Eq`, `Ne` | Abstract equality (`==`, `!=`) |
| `StrictEq`, `StrictNe` | Strict equality (`===`, `!==`) |
| `Lt`, `Le`, `Gt`, `Ge` | Relational comparison |
| `Not` | Logical NOT |

### Control Flow

| Op | Description |
|----|-------------|
| `Jump(i32)` | Unconditional relative jump |
| `JumpIfTrue(i32)` | Jump if truthy (pops) |
| `JumpIfFalse(i32)` | Jump if falsy (pops) |
| `JumpIfNullish(i32)` | Jump if nullish (does not pop) |

### Functions

| Op | Description |
|----|-------------|
| `Call(u8)` | Call function with N arguments |
| `CallMethod(u8)` | Call method with this binding |
| `Return` | Return from function |
| `Closure(u16)` | Create closure from function prototype |
| `LoadThis` | Push current `this` binding |

### Objects and Properties

| Op | Description |
|----|-------------|
| `GetProp` | Get property: `[obj, key]` -> `[value]` |
| `SetProp` | Set property: `[obj, key, value]` -> `[value]` |
| `GetPropNamed(u16)` | Get property by constant name |
| `SetPropNamed(u16)` | Set property by constant name |
| `NewObject` | Create empty object |
| `NewArray(u16)` | Create array from N stack values |
| `New(u8)` | Constructor call (`new Foo(args)`) |

### Special Operators

| Op | Description |
|----|-------------|
| `Typeof` | Push type string |
| `Void` | Replace top with `undefined` |
| `Delete` | Delete property: `[obj, key]` -> `[bool]` |
| `InstanceOf` | `instanceof` check |
| `In` | `in` operator |

### Iteration

| Op | Description |
|----|-------------|
| `GetIterator` | Create iterator from iterable |
| `IterNext` | Advance iterator, push `[value, has_more]` |

### Spread

| Op | Description |
|----|-------------|
| `Spread` | Spread iterable elements into target array |
| `ArrayPush` | Push single value into array below it |

### Exception Handling

| Op | Description |
|----|-------------|
| `TryCatch(i32, i32)` | Set up try-catch (catch offset, finally offset) |
| `TryEnd` | End try/catch block |
| `Throw` | Throw exception from top of stack |

### Async

| Op | Description |
|----|-------------|
| `Await` | Await a value (unwraps fulfilled promises, throws rejected ones) |

### Misc

| Op | Description |
|----|-------------|
| `Debugger` | No-op (debugger statement) |
| `Nop` | No-op |

---

## Native Function Signature

All native functions registered with the engine must have this signature:

```rust
fn my_native(vm: &mut Vm, args: &[JsValue]) -> JsValue
```

- `vm` -- The VM instance, giving access to globals, stack, console output, prototypes, and `userdata`.
- `args` -- The arguments passed from JavaScript.
- Returns a `JsValue` that becomes the function's return value in JS.

### Example: Registering a Native Function

```rust
use libjs::{JsEngine, JsValue, Vm};

fn js_add_numbers(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let a = args.get(0).map(|v| v.to_number()).unwrap_or(0.0);
    let b = args.get(1).map(|v| v.to_number()).unwrap_or(0.0);
    JsValue::Number(a + b)
}

let mut engine = JsEngine::new();
engine.register_native("addNumbers", js_add_numbers);

let result = engine.eval("addNumbers(10, 20)");
// result.to_number() == 30.0
```

---

## Examples

### Evaluate and Inspect

```rust
use libjs::JsEngine;

let mut engine = JsEngine::new();

// Arithmetic
let val = engine.eval("(2 + 3) * 4 - 1");
assert_eq!(val.to_number(), 19.0);

// String operations
let val = engine.eval("'Hello'.toUpperCase() + ' WORLD'");
assert_eq!(val.to_js_string(), "HELLO WORLD");

// Arrays
let val = engine.eval("[1, 2, 3].map(x => x * 2).join(', ')");
assert_eq!(val.to_js_string(), "2, 4, 6");
```

### Bidirectional Data Exchange

```rust
use libjs::{JsEngine, JsValue};

let mut engine = JsEngine::new();

// Pass data into JS
engine.set_global("width", JsValue::Number(800.0));
engine.set_global("height", JsValue::Number(600.0));
engine.set_global("title", JsValue::String(String::from("My Window")));

// Compute in JS, read back
let area = engine.eval("width * height");
assert_eq!(area.to_number(), 480000.0);
```

### Compile and Execute Separately

```rust
use libjs::JsEngine;

let mut engine = JsEngine::new();

// Compile once
let chunk = engine.compile("function square(n) { return n * n; } square(7);");

// Execute the compiled bytecode
let result = engine.vm().execute(chunk);
assert_eq!(result.to_number(), 49.0);
```

### Capture Console Output

```rust
use libjs::JsEngine;

let mut engine = JsEngine::new();

engine.eval(r#"
    var items = ['apple', 'banana', 'cherry'];
    items.forEach(function(item, i) {
        console.log(i + ': ' + item);
    });
"#);

for msg in engine.console_output() {
    // "0: apple", "1: banana", "2: cherry"
}

engine.clear_console();
```

### Prevent Infinite Loops

```rust
use libjs::JsEngine;

let mut engine = JsEngine::new();
engine.set_step_limit(1000);

// This will be aborted after 1000 VM steps
let result = engine.eval("while(true) {}");
// result is Undefined (execution aborted)
```

### Extending Built-in Prototypes

```rust
use libjs::{JsEngine, JsValue, Vm};
use libjs::vm::native_fn;

fn string_reverse(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let JsValue::String(s) = &vm.current_this {
        let reversed: String = s.chars().rev().collect();
        JsValue::String(reversed)
    } else {
        JsValue::Undefined
    }
}

let mut engine = JsEngine::new();

// Add reverse() to String.prototype
{
    let vm = engine.vm();
    let proto = vm.string_proto.clone();
    proto.borrow_mut().set(
        String::from("reverse"),
        native_fn("reverse", string_reverse),
    );
}

let val = engine.eval("'hello'.reverse()");
// val.to_js_string() == "olleh"
```
