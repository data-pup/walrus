//! Memories used in a wasm module.

use crate::emit::{Emit, EmitContext, Section};
use crate::ir::Value;
use crate::parse::IndicesToIds;
use crate::tombstone_arena::{Id, Tombstone, TombstoneArena};
use crate::{GlobalId, ImportId, InitExpr, Module, Result};

/// The id of a memory.
pub type MemoryId = Id<Memory>;

/// A memory in the wasm.
#[derive(Debug)]
pub struct Memory {
    id: MemoryId,
    /// Is this memory shared?
    pub shared: bool,
    /// The initial page size for this memory
    pub initial: u32,
    /// The maximum page size for this memory
    pub maximum: Option<u32>,
    /// Whether or not this memory is imported, and if so from where
    pub import: Option<ImportId>,
    /// Data that will be used to initialize this memory chunk, with known
    /// static offsets
    pub data: MemoryData,
}

impl Tombstone for Memory {
    fn on_delete(&mut self) {
        self.data = MemoryData::default();
    }
}

/// An abstraction for the initialization values of a `Memory`.
///
/// This houses all the data sections of a wasm executable that as associated
/// with this `Memory`.
#[derive(Debug, Default)]
pub struct MemoryData {
    absolute: Vec<(u32, Vec<u8>)>,
    relative: Vec<(GlobalId, Vec<u8>)>,
}

impl Memory {
    /// Return the id of this memory
    pub fn id(&self) -> MemoryId {
        self.id
    }

    pub(crate) fn emit_data(&self) -> impl Iterator<Item = (InitExpr, &[u8])> {
        let absolute = self
            .data
            .absolute
            .iter()
            .map(move |(pos, data)| (InitExpr::Value(Value::I32(*pos as i32)), &data[..]));
        let relative = self
            .data
            .relative
            .iter()
            .map(move |(id, data)| (InitExpr::Global(*id), &data[..]));
        absolute.chain(relative)
    }
}

impl Emit for Memory {
    fn emit(&self, cx: &mut EmitContext) {
        if let Some(max) = self.maximum {
            cx.encoder.byte(if self.shared { 0x03 } else { 0x01 });
            cx.encoder.u32(self.initial);
            cx.encoder.u32(max);
        } else {
            cx.encoder.byte(0x00);
            cx.encoder.u32(self.initial);
        }
    }
}

/// The set of memories in this module.
#[derive(Debug, Default)]
pub struct ModuleMemories {
    arena: TombstoneArena<Memory>,
}

impl ModuleMemories {
    /// Add an imported memory
    pub fn add_import(
        &mut self,
        shared: bool,
        initial: u32,
        maximum: Option<u32>,
        import: ImportId,
    ) -> MemoryId {
        let id = self.arena.next_id();
        let id2 = self.arena.alloc(Memory {
            id,
            shared,
            initial,
            maximum,
            import: Some(import),
            data: MemoryData::default(),
        });
        debug_assert_eq!(id, id2);
        id
    }

    /// Construct a new memory, that does not originate from any of the input
    /// wasm memories.
    pub fn add_local(&mut self, shared: bool, initial: u32, maximum: Option<u32>) -> MemoryId {
        let id = self.arena.next_id();
        let id2 = self.arena.alloc(Memory {
            id,
            shared,
            initial,
            maximum,
            import: None,
            data: MemoryData::default(),
        });
        debug_assert_eq!(id, id2);
        id
    }

    /// Gets a reference to a memory given its id
    pub fn get(&self, id: MemoryId) -> &Memory {
        &self.arena[id]
    }

    /// Gets a reference to a memory given its id
    pub fn get_mut(&mut self, id: MemoryId) -> &mut Memory {
        &mut self.arena[id]
    }

    /// Removes a memory from this module.
    ///
    /// It is up to you to ensure that any potential references to the deleted
    /// memory are also removed, eg `mem.load` expressions and exports.
    pub fn delete(&mut self, id: MemoryId) {
        self.arena.delete(id);
    }

    /// Get a shared reference to this module's memories.
    pub fn iter(&self) -> impl Iterator<Item = &Memory> {
        self.arena.iter().map(|(_, f)| f)
    }

    /// Get a mutable reference to this module's memories.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Memory> {
        self.arena.iter_mut().map(|(_, f)| f)
    }
}

impl Module {
    /// Construct a new, empty set of memories for a module.
    pub(crate) fn parse_memories(
        &mut self,
        section: wasmparser::MemorySectionReader,
        ids: &mut IndicesToIds,
    ) -> Result<()> {
        log::debug!("parse memory section");
        for m in section {
            let m = m?;
            let id = self
                .memories
                .add_local(m.shared, m.limits.initial, m.limits.maximum);
            ids.push_memory(id);
        }
        Ok(())
    }
}

impl Emit for ModuleMemories {
    fn emit(&self, cx: &mut EmitContext) {
        log::debug!("emit memory section");
        // imported memories are emitted earlier
        let memories = self.iter().filter(|m| m.import.is_none()).count();
        if memories == 0 {
            return;
        }

        let mut cx = cx.start_section(Section::Memory);
        cx.encoder.usize(memories);
        for memory in self.iter().filter(|m| m.import.is_none()) {
            cx.indices.push_memory(memory.id());
            memory.emit(&mut cx);
        }
    }
}

impl MemoryData {
    /// Adds a new chunk of data in this `ModuleData` at an absolute address
    pub fn add_absolute(&mut self, pos: u32, data: Vec<u8>) {
        self.absolute.push((pos, data));
    }

    /// Adds a new chunk of data in this `ModuleData` at a relative address
    pub fn add_relative(&mut self, id: GlobalId, data: Vec<u8>) {
        self.relative.push((id, data));
    }

    /// Returns an iterator of all globals used as relative bases
    pub fn globals<'a>(&'a self) -> impl Iterator<Item = GlobalId> + 'a {
        self.relative.iter().map(|p| p.0)
    }

    /// Returns whether this data has no initialization sections
    pub fn is_empty(&self) -> bool {
        self.absolute.is_empty() && self.relative.is_empty()
    }

    /// Consumes this data and returns a by-value iterator of each segment
    pub fn into_iter(self) -> impl Iterator<Item = (InitExpr, Vec<u8>)> {
        let absolute = self
            .absolute
            .into_iter()
            .map(move |(pos, data)| (InitExpr::Value(Value::I32(pos as i32)), data));
        let relative = self
            .relative
            .into_iter()
            .map(move |(id, data)| (InitExpr::Global(id), data));
        absolute.chain(relative)
    }
}
