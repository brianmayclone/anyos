//! JavaScript bytecode definitions.
//!
//! Opcodes for a stack-based virtual machine.

use alloc::string::String;
use alloc::vec::Vec;

/// A single bytecode instruction.
#[derive(Debug, Clone)]
pub enum Op {
    // ── Stack Operations ──
    /// Push a constant from the constant pool.
    LoadConst(u16),
    /// Push undefined.
    LoadUndefined,
    /// Push null.
    LoadNull,
    /// Push true.
    LoadTrue,
    /// Push false.
    LoadFalse,
    /// Pop and discard the top value.
    Pop,
    /// Duplicate the top value.
    Dup,

    // ── Variable Operations ──
    /// Load a local variable by slot index.
    LoadLocal(u16),
    /// Store top of stack into a local variable slot.
    StoreLocal(u16),
    /// Load a global variable by name (constant pool index).
    /// Throws ReferenceError if the variable is not defined.
    LoadGlobal(u16),
    /// Load a global variable by name, returning undefined if not defined.
    /// Used by `typeof` to avoid ReferenceError on undeclared variables.
    LoadGlobalSafe(u16),
    /// Store top of stack into a global variable.
    StoreGlobal(u16),
    /// Load from closure (upvalue) — (upvalue_index).
    LoadUpvalue(u16),
    /// Store into closure (upvalue).
    StoreUpvalue(u16),

    // ── Arithmetic ──
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Exp, // **
    Neg, // unary -
    Pos, // unary +

    // ── Bitwise ──
    BitAnd,
    BitOr,
    BitXor,
    BitNot,
    Shl,
    Shr,
    UShr,

    // ── Comparison ──
    Eq,       // ==
    Ne,       // !=
    StrictEq, // ===
    StrictNe, // !==
    Lt,       // <
    Le,       // <=
    Gt,       // >
    Ge,       // >=

    // ── Logical ──
    Not, // !

    // ── Control Flow ──
    /// Unconditional jump (relative offset).
    Jump(i32),
    /// Jump if top of stack is truthy (pops).
    JumpIfTrue(i32),
    /// Jump if top of stack is falsy (pops).
    JumpIfFalse(i32),
    /// Jump if top of stack is nullish (does NOT pop).
    JumpIfNullish(i32),

    // ── Functions ──
    /// Call a function: Call(arg_count). Callee is below args on stack.
    Call(u8),
    /// Method call: CallMethod(arg_count).
    /// Stack: [..., this_obj, method_fn, arg1, ..., argN]
    /// Pops args, method, and this; calls method with this binding.
    CallMethod(u8),
    /// Return from function. Return value is on top of stack.
    Return,
    /// Create a closure from a function prototype (constant pool index).
    Closure(u16),

    // ── Objects and Properties ──
    /// Get property: stack = [..., object, key] → [..., value]
    GetProp,
    /// Set property: stack = [..., object, key, value] → [..., value]
    SetProp,
    /// Get property by name (constant pool index): stack = [..., object] → [..., value]
    GetPropNamed(u16),
    /// Set property by name: stack = [..., object, value] → [..., value]
    SetPropNamed(u16),
    /// Create new empty object.
    NewObject,
    /// Create new array with N elements from stack.
    NewArray(u16),

    // ── Constructors ──
    /// new Constructor(args): New(arg_count). Constructor is below args.
    New(u8),

    // ── Special ──
    /// typeof operator. Pops value, pushes type string.
    Typeof,
    /// void operator. Pops value, pushes undefined.
    Void,
    /// delete operator on property: stack = [..., object, key] → [..., bool]
    Delete,
    /// instanceof operator: stack = [..., left, right] → [..., bool]
    InstanceOf,
    /// in operator: stack = [..., key, object] → [..., bool]
    In,

    // ── Iteration ──
    /// Get iterator from iterable on top of stack.
    GetIterator,
    /// Advance iterator: pushes { value, done }.
    IterNext,

    // ── Exception Handling ──
    /// Set up try-catch: TryCatch(catch_offset, finally_offset).
    /// Offsets are relative to current position. 0 means no catch/finally.
    TryCatch(i32, i32),
    /// End of try/catch/finally block.
    TryEnd,
    /// Throw exception. Value is on top of stack.
    Throw,

    // ── Increment/Decrement ──
    /// Pre-increment: load, add 1, store, push new value.
    Inc,
    /// Pre-decrement: load, sub 1, store, push new value.
    Dec,

    // ── Special Assignments ──
    /// Compound assignment: reads variable, applies op, stores back.
    /// AddAssign etc. are compiled as load + op + store sequences.

    // ── This ──
    /// Push `this` binding.
    LoadThis,

    // ── Spread/Rest ──
    /// Spread array-or-iterable into target array.
    /// Stack before: [..., target_array, value_to_spread]
    /// Stack after:  [..., target_array]  (value's elements appended to target)
    Spread,
    /// Spread an object's own enumerable properties into a target object.
    /// Stack before: [..., target_object, source_object]
    /// Stack after:  [..., target_object]  (source's properties copied to target)
    ObjectSpread,
    /// Push a single value into an array that sits below it on the stack.
    /// Stack before: [..., target_array, value]
    /// Stack after:  [..., target_array]  (value appended to target)
    ArrayPush,
    /// Build an array from all call arguments starting at index `n`.
    /// Stack before: [...] → Stack after: [..., Array(args[n..])]
    LoadArgsArray(u16),
    /// Call a function with arguments supplied as an array on the stack.
    /// Stack before: [..., callee, args_array]
    /// Stack after:  [..., return_value]
    CallSpread,
    /// Call a method with arguments supplied as an array on the stack.
    /// Stack before: [..., this_obj, method_fn, args_array]
    /// Stack after:  [..., return_value]
    CallMethodSpread,
    /// Load the currently-executing function value (for named function expressions).
    LoadSelf,

    // ── Debugger ──
    Debugger,

    // ── Async ──
    /// Await a value: if it's a fulfilled Promise, push its resolved value.
    /// If it's a rejected Promise, throw the rejection reason.
    /// If it's a non-Promise value, push it back unchanged.
    Await,

    /// Yield a value from a generator function.
    /// Suspends execution and returns `{ value, done: false }` to the caller.
    /// When resumed via `.next(v)`, `v` replaces the yield expression result.
    Yield,

    /// Yield delegate: `yield* iterable`.
    /// Delegates to another iterable/generator.
    YieldDelegate,

    // ── No-op ──
    Nop,

    /// Create a rest object from the source object on top of stack by copying all
    /// own enumerable properties except for `n` excluded keys (also on stack).
    /// Stack before: [..., source_obj, key1, ..., keyN]
    /// Stack after:  [..., rest_object]
    ObjectRest(u8),

    /// Create a RegExp object from pattern (const index) and flags (const index).
    /// Stack before: [...]
    /// Stack after:  [..., regexp_object]
    NewRegExp(u16, u16),

    /// Define a getter on an object.
    /// Stack before: [..., object, getter_fn]
    /// Stack after:  [..., object]
    DefineGetter(u16), // name constant index

    /// Define a getter on an object with a computed key.
    /// Stack before: [..., object, key, getter_fn]
    /// Stack after:  [..., object]
    DefineGetterComputed,

    /// Define a setter on an object.
    /// Stack before: [..., object, setter_fn]
    /// Stack after:  [..., object]
    DefineSetter(u16), // name constant index

    /// Define a setter on an object with a computed key.
    /// Stack before: [..., object, key, setter_fn]
    /// Stack after:  [..., object]
    DefineSetterComputed,

    /// Define a method on an object (non-enumerable, writable, configurable).
    /// Like SetPropNamed but creates a non-enumerable property (ES2023 §14.5.14).
    /// Stack before: [..., object, method_fn]
    /// Stack after:  [..., method_fn]
    DefineMethod(u16), // name constant index

    /// Replace local slot `slot` with a fresh `Rc<RefCell<JsValue>>` cell containing
    /// the same value.  Used for per-iteration `let` binding in `for` loops so that
    /// closures created in each iteration capture their own cell.
    CloneLocal(u16),

    /// Push the `new.target` meta-property value.
    /// Inside a constructor call (`new Foo()`), pushes the constructor function.
    /// Outside a constructor, pushes `undefined`.
    NewTarget,

    /// Call super constructor: `super(args)` in a derived class constructor.
    /// Stack before: [..., SuperClass, arg1, ..., argN]
    /// Sets `is_constructor = true` and forwards `new.target` from the current frame.
    SuperCall(u8),
    /// Set the super_class field on the top-of-stack Function.
    /// Stack: [..., Constructor, SuperClass] → [..., Constructor]
    SetSuperClass,

    /// ES2023 §7.2.1 RequireObjectCoercible(value).
    /// Throws TypeError if the value on top of stack is null or undefined.
    /// Does not pop the value.
    RequireObjectCoercible,
}

/// Describes how a compiled function captures one upvalue from its enclosing scope.
#[derive(Debug, Clone)]
pub struct UpvalueRef {
    /// If `true`, capture local slot `index` from the immediately enclosing function's locals.
    /// If `false`, capture upvalue `index` from the immediately enclosing function's upvalue list.
    pub is_local: bool,
    /// Slot index (local or upvalue) in the immediately enclosing function.
    pub index: u16,
}

/// A compiled function / code block.
#[derive(Debug, Clone)]
pub struct Chunk {
    /// Bytecode instructions.
    pub code: Vec<Op>,
    /// Constant pool (strings, numbers, nested functions).
    pub constants: Vec<Constant>,
    /// Number of local variable slots needed.
    pub local_count: u16,
    /// Number of parameters.
    pub param_count: u16,
    /// Function name (for debugging).
    pub name: Option<String>,
    /// Upvalue capture descriptors — one entry per upvalue the function closes over.
    pub upvalues: Vec<UpvalueRef>,
    /// True if this chunk was compiled under `"use strict"` mode.
    pub strict: bool,
    /// True if this is a generator function (`function*`).
    pub is_generator: bool,
    /// True if this is an arrow function.
    /// Arrow functions lexically capture `this` from their enclosing scope
    /// (ES6 §14.2.16) — they do NOT have their own `this` binding.
    pub is_arrow: bool,
    /// True if this is an async function.
    pub is_async: bool,
}

impl Chunk {
    pub fn new() -> Self {
        Chunk {
            code: Vec::new(),
            constants: Vec::new(),
            local_count: 0,
            param_count: 0,
            name: None,
            upvalues: Vec::new(),
            strict: false,
            is_generator: false,
            is_arrow: false,
            is_async: false,
        }
    }

    /// Add a constant and return its index.
    pub fn add_const(&mut self, c: Constant) -> u16 {
        // Check for duplicates
        for (i, existing) in self.constants.iter().enumerate() {
            if existing.eq_value(&c) {
                return i as u16;
            }
        }
        let idx = self.constants.len() as u16;
        self.constants.push(c);
        idx
    }

    /// Emit an instruction and return its index.
    pub fn emit(&mut self, op: Op) -> usize {
        let idx = self.code.len();
        self.code.push(op);
        idx
    }

    /// Current code offset (next instruction index).
    pub fn offset(&self) -> usize {
        self.code.len()
    }

    /// Patch a jump instruction at `idx` to jump to current offset.
    pub fn patch_jump(&mut self, idx: usize) {
        self.patch_jump_to_pos(idx, self.code.len());
    }

    /// Patch a jump instruction at `idx` to jump to a specific absolute position.
    pub fn patch_jump_to_pos(&mut self, idx: usize, pos: usize) {
        let target = pos as i32 - idx as i32 - 1;
        match &mut self.code[idx] {
            Op::Jump(ref mut off) => *off = target,
            Op::JumpIfTrue(ref mut off) => *off = target,
            Op::JumpIfFalse(ref mut off) => *off = target,
            Op::JumpIfNullish(ref mut off) => *off = target,
            _ => {}
        }
    }
}

/// Constant pool entry.
#[derive(Debug, Clone)]
pub enum Constant {
    Number(f64),
    String(String),
    /// A nested function prototype.
    Function(Chunk),
}

impl Constant {
    fn eq_value(&self, other: &Constant) -> bool {
        match (self, other) {
            (Constant::Number(a), Constant::Number(b)) => a.to_bits() == b.to_bits(),
            (Constant::String(a), Constant::String(b)) => a == b,
            _ => false,
        }
    }
}
