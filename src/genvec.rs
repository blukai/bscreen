use std::{
    hash::{Hash, Hasher},
    marker::PhantomData,
    num::NonZeroU32,
};

/// Tracks the generation of an entry in [`GenVec`].
///
/// The idea to use `NonZeroU32` is borrowed from [thunderdome][1].
///
/// [1]: https://github.com/LPGhatguy/thunderdome/blob/9e0a6dc3d2e6d402a2f985e47a876156d42c198b/src/generation.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
struct Generation(NonZeroU32);

impl Generation {
    /// Useful for two-phase initialization.
    const DANGLING: Self = Self(unsafe { NonZeroU32::new_unchecked(u32::MAX) });

    #[expect(dead_code)]
    #[inline]
    fn is_dangling(&self) -> bool {
        self.eq(&Self::DANGLING)
    }

    #[inline]
    fn new() -> Self {
        Self(unsafe { NonZeroU32::new_unchecked(1) })
    }

    #[inline]
    fn try_bump(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

/// A non-owning, cheap-to-copy reference to an entry in a [`GenVec`].
pub struct Handle<T> {
    index: u32,
    generation: Generation,
    type_marker: PhantomData<T>,
}

// NOTE: traits down below for `Handle` are not derived, but implemented by hand because deriving
// these traits would require `T` to implement them, which is unnecessary since `T` is merely a type
// marker.

impl<T> std::fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Handle")
            .field("index", &self.index)
            .field("generation", &self.generation)
            .field("type_marker", &std::any::type_name::<T>())
            .finish()
    }
}

impl<T> Clone for Handle<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Handle<T> {}

impl<T> PartialEq for Handle<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}

impl<T> Eq for Handle<T> {}

impl<T> Hash for Handle<T> {
    // NOTE: this is very non collision free hash
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        self.generation.hash(state);
    }
}

impl<T> Default for Handle<T> {
    #[inline]
    fn default() -> Self {
        Self::DANGLING
    }
}

// NOTE: handles don't carry data. it is safet to send/share them between threads.
unsafe impl<T> Send for Handle<T> {}
unsafe impl<T> Sync for Handle<T> {}

impl<T> Handle<T> {
    /// Useful for two-phase initialization.
    ///
    /// In two-phase initialization, a dangling handle is created first, and later replaced
    /// with a valid handle after the associated entry has been initialized.
    pub const DANGLING: Self = Self {
        index: 0,
        generation: Generation::DANGLING,
        type_marker: PhantomData,
    };

    #[inline]
    pub fn is_dangling(&self) -> bool {
        self.eq(&Self::DANGLING)
    }

    #[inline]
    fn new(index: u32, generation: Generation) -> Self {
        Self {
            index,
            generation,
            type_marker: PhantomData,
        }
    }
}

// NOTE: the entry is occupied when value is `Some`, vaccant if it's `None`.
#[derive(Debug)]
struct Entry<T> {
    generation: Generation,
    value: Option<T>,
}

/// A reference to a reserved entry in a [`GenVec`].
pub struct Ticket<T> {
    index: u32,
    type_marker: PhantomData<T>,
}

impl<T> Drop for Ticket<T> {
    fn drop(&mut self) {
        panic!("a thing must be returned to the GenVec it was taken from!");
    }
}

impl<T> Ticket<T> {
    #[inline]
    fn new(index: u32) -> Self {
        Self {
            index,
            type_marker: PhantomData,
        }
    }
}

/// An encapsulated Vec that allows to refer to entries by [`Handle`].
///
/// Methods with the `try_` prefix return `Option`, allowing for graceful error handling. These
/// methods are suitable when failures are expected and should be handled without crashing the
/// program.
///
/// Methods without the `try_` prefix can panic. They assume valid input and are intended for cases
/// where failure is considered a logic error or a violation of preconditions, which should never
/// occur under normal circumstances.
///
/// This is an attempt to align with Rust's philosophy of making failure explicit and providing a
/// way to handle errors in a controlled manner, while also allowing for performance optimizations
/// and simpler code paths when failures are truly unexpected.
///
/// ## reading:
///
/// - <https://floooh.github.io/2018/06/17/handles-vs-pointers.html>
/// - <https://verdagon.dev/blog/generational-references>
///
/// ## alternatives:
///
/// - <https://github.com/orlp/slotmap>
/// - <https://github.com/LPGhatguy/thunderdome>
/// - <https://github.com/fitzgen/generational-arena>
/// - <https://docs.rs/fyrox/latest/fyrox/core/pool/struct.Pool.html>
#[derive(Debug)]
pub struct GenVec<T> {
    entries: Vec<Entry<T>>,
    // TODO: is there a better/cheaper way to keep track of free indices? Vecs are fat!
    free_indices: Vec<u32>,
}

impl<T> Default for GenVec<T> {
    fn default() -> Self {
        Self {
            entries: vec![],
            free_indices: vec![],
        }
    }
}

// NOTE: in rust there's no idiomatic/conventional naming for methods that panic or return option;
// it is common to add prefix try_ to methods that return option.
//
// be aware that methods that get
// values out of the GenVec and are not prefixed with try_ can panic!

impl<T> GenVec<T> {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
            ..Default::default()
        }
    }

    #[inline]
    fn try_get_entry_by_index(&self, index: u32) -> Option<&Entry<T>> {
        self.entries.get(index as usize)
    }

    #[expect(dead_code)]
    #[inline]
    fn get_entry_by_index(&self, index: u32) -> &Entry<T> {
        self.try_get_entry_by_index(index)
            .unwrap_or_else(|| panic!("could not get entry at index {}", index))
    }

    #[inline]
    fn try_get_entry_by_index_mut(&mut self, index: u32) -> Option<&mut Entry<T>> {
        self.entries.get_mut(index as usize)
    }

    #[inline]
    fn get_entry_by_index_mut(&mut self, index: u32) -> &mut Entry<T> {
        self.try_get_entry_by_index_mut(index)
            .unwrap_or_else(|| panic!("could not get entry at index {}", index))
    }

    #[inline]
    fn try_get_entry_by_handle(&self, handle: Handle<T>) -> Option<&Entry<T>> {
        if let Some(entry) = self.try_get_entry_by_index(handle.index) {
            if entry.generation == handle.generation {
                return Some(entry);
            }
        }
        None
    }

    #[expect(dead_code)]
    #[inline]
    fn get_entry_by_handle(&self, handle: Handle<T>) -> &Entry<T> {
        self.try_get_entry_by_handle(handle)
            .unwrap_or_else(|| panic!("could not get entry at handle {:?}", handle))
    }

    #[inline]
    fn try_get_entry_by_handle_mut(&mut self, handle: Handle<T>) -> Option<&mut Entry<T>> {
        if let Some(entry) = self.try_get_entry_by_index_mut(handle.index) {
            if entry.generation == handle.generation {
                return Some(entry);
            }
        }
        None
    }

    #[inline]
    fn get_entry_by_handle_mut(&mut self, handle: Handle<T>) -> &mut Entry<T> {
        self.try_get_entry_by_handle_mut(handle)
            .unwrap_or_else(|| panic!("could not get entry at handle {:?}", handle))
    }

    pub fn try_get(&self, handle: Handle<T>) -> Option<&T> {
        self.try_get_entry_by_handle(handle).and_then(|entry| entry.value.as_ref())
    }

    pub fn get(&self, handle: Handle<T>) -> &T {
        // TODO: get must not call try_get.
        self.try_get(handle).unwrap_or_else(|| panic!("could not get at handle {:?}", handle))
    }

    pub fn try_get_mut(&mut self, handle: Handle<T>) -> Option<&mut T> {
        self.try_get_entry_by_handle_mut(handle).and_then(|entry| entry.value.as_mut())
    }

    pub fn get_mut(&mut self, handle: Handle<T>) -> &mut T {
        // TODO: get_mut must not call try_get_mut.
        self.try_get_mut(handle).unwrap_or_else(|| panic!("could not get at handle {:?}", handle))
    }

    /// Construct a value with the handle it would be given. The handle is _not_ valid until
    /// function has finished executing.
    pub fn insert_with(&mut self, callback: impl FnOnce(Handle<T>) -> T) -> Handle<T> {
        // NOTE: loop to find a valid (not overflowed) free index
        while let Some(index) = self.free_indices.pop() {
            let entry = self.get_entry_by_index_mut(index);

            if entry.value.is_some() {
                panic!("attempt to insert into non-freed entry at index {}", index);
            }

            // QUOTE: Once the generation counter would ‘overflow’, disable that array slot, so that
            // no new handles are returned for this slot.
            // https://floooh.github.io/2018/06/17/handles-vs-pointers.html
            let Some(generation) = entry.generation.try_bump() else {
                continue;
            };
            let handle = Handle::new(index, generation);

            entry.generation = generation;
            entry.value.replace(callback(handle));

            return handle;
        }

        let handle = Handle::new(self.entries.len() as u32, Generation::new());

        self.entries.push(Entry {
            value: Some(callback(handle)),
            generation: handle.generation,
        });

        handle
    }

    #[inline]
    pub fn insert(&mut self, value: T) -> Handle<T> {
        self.insert_with(|_| value)
    }

    // TODO: non-panicking try_remove

    pub fn remove(&mut self, handle: Handle<T>) -> T {
        let entry = self.get_entry_by_handle_mut(handle);

        let value = entry
            .value
            .take()
            .unwrap_or_else(|| panic!("attempt to double free entry at handle {:?}", handle));

        self.free_indices.push(handle.index);

        value
    }

    /// Tries to take ownership of the value at the given handle.
    ///
    /// Returns a [`Ticket`] representing a temporary reservation of an entry, along with the owned
    /// value, or `None` if the given handle is invalid or entry is not occupied.
    ///
    /// All existing handles pointing to the entry will be invalid until the value is returned
    /// using the [`put_back`] method.
    ///
    /// If you lose the [`Ticket`], the entry will remain unusable forever.
    ///
    /// [`put_back`]: GenVec::put_back
    pub fn try_take(&mut self, handle: Handle<T>) -> Option<(Ticket<T>, T)> {
        if let Some(entry) = self.try_get_entry_by_handle_mut(handle) {
            if let Some(value) = entry.value.take() {
                return Some((Ticket::new(handle.index), value));
            }
        }
        None
    }

    /// Same as [`try_take`], but panics if handle is invalid.
    ///
    /// [`try_take`]: GenVec::try_take
    #[inline]
    pub fn take(&mut self, handle: Handle<T>) -> (Ticket<T>, T) {
        // TODO: take must not call try_take.
        self.try_take(handle)
            .unwrap_or_else(|| panic!("could not take value at handle {:?}", handle))
    }

    /// Puts back the value into the entry associated with the given [`Ticket`] that was previously
    /// obtained with [`try_take`] or [`take`]. See [`try_take`] for more info.
    ///
    /// [`try_take`]: GenVec::try_take
    /// [`take`]: GenVec::take
    pub fn put_back(&mut self, ticket: Ticket<T>, value: T) {
        let entry = self.get_entry_by_index_mut(ticket.index);
        entry.value.replace(value);
        // NOTE: forget is called to not invoke manually implemented panicking drop.
        std::mem::forget(ticket);
    }

    pub fn iter(&self) -> impl Iterator<Item = (Handle<T>, &T)> {
        self.entries.iter().enumerate().filter_map(|(index, entry)| {
            entry.value.as_ref().map(|value| (Handle::new(index as u32, entry.generation), value))
        })
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Handle<T>, &mut T)> {
        self.entries.iter_mut().enumerate().filter_map(|(index, entry)| {
            entry.value.as_mut().map(|value| (Handle::new(index as u32, entry.generation), value))
        })
    }

    pub fn iter_values(&self) -> impl Iterator<Item = &T> {
        self.entries.iter().filter_map(|entry| entry.value.as_ref())
    }

    pub fn iter_values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.entries.iter_mut().filter_map(|entry| entry.value.as_mut())
    }

    /// Returnes the number of entries (both occupied and vaccant).
    #[inline]
    pub fn len(&self) -> u32 {
        u32::try_from(self.entries.len()).unwrap_or_else(|_| panic!("entries.len() overflored u32"))
    }

    /// Returns a potentially dangling `Handle` for the entry at the given index.
    pub fn handle_from_index(&self, index: u32) -> Handle<T> {
        if let Some(entry) = self.try_get_entry_by_index(index) {
            return Handle::new(index, entry.generation);
        }
        Handle::DANGLING
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_remove() {
        let mut gv = GenVec::default();
        let handle = gv.insert("hello");

        assert_eq!(gv.entries.len(), 1);
        assert_eq!(gv.free_indices.len(), 0);

        let res = gv.remove(handle);

        assert_eq!(res, "hello");
        assert_eq!(gv.entries.len(), 1);
        assert_eq!(gv.free_indices.len(), 1);
    }

    #[test]
    #[should_panic]
    fn test_remove_at_invalid_handle() {
        let mut gv: GenVec<()> = GenVec::default();

        let handle = Handle::DANGLING;

        gv.remove(handle);
    }

    #[test]
    fn test_take_and_put_back() {
        let mut gv = GenVec::default();
        let handle = gv.insert(42u8);

        let (ticket, value) = gv.take(handle);
        assert_eq!(std::any::type_name_of_val(&value), "u8");

        gv.put_back(ticket, value);
    }

    #[test]
    #[should_panic]
    fn test_drop_ticket_without_put_back() {
        let mut gv = GenVec::default();
        let handle = gv.insert("hello");
        gv.take(handle);
    }
}
