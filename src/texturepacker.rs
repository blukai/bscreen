use crate::{
    genvec::Handle,
    ntree::{NTree, NTreeNode},
};

// TODO: do not use dangling handles in public api, but Option<Handle<T>> instead.

pub const DEFAULT_TEXTURE_WIDTH: u32 = 1024;
pub const DEFAULT_TEXTURE_HEIGHT: u32 = 1024;

// TODO: consider using word "region"
#[derive(Debug)]
pub struct TexturePackerEntry {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,

    in_use: bool,
}

/// manages texture packing of textures as they are added.
#[derive(Debug)]
pub struct TexturePacker {
    w: u32,
    h: u32,

    ntree: NTree<TexturePackerEntry>,
}

impl Default for TexturePacker {
    fn default() -> Self {
        Self {
            w: DEFAULT_TEXTURE_WIDTH,
            h: DEFAULT_TEXTURE_HEIGHT,

            ntree: NTree::new(TexturePackerEntry {
                x: 0,
                y: 0,
                w: DEFAULT_TEXTURE_WIDTH,
                h: DEFAULT_TEXTURE_HEIGHT,

                in_use: false,
            }),
        }
    }
}

impl TexturePacker {
    pub fn new(texture_width: u32, texture_height: u32) -> Self {
        Self {
            w: texture_width,
            h: texture_height,

            ntree: NTree::new(TexturePackerEntry {
                x: 0,
                y: 0,
                w: texture_width,
                h: texture_height,

                in_use: false,
            }),
        }
    }

    fn is_leaf(&self, handle: Handle<NTreeNode<TexturePackerEntry>>) -> bool {
        self.ntree
            .get(handle)
            .first_child
            .is_none_or(|h| self.ntree.get(h).next_sibling.is_none())
    }

    fn is_left_child(
        &self,
        parent_handle: Handle<NTreeNode<TexturePackerEntry>>,
        child_handle: Handle<NTreeNode<TexturePackerEntry>>,
    ) -> bool {
        self.ntree
            .get(parent_handle)
            .first_child
            .is_some_and(|h| h == child_handle)
    }

    fn is_right_child(
        &self,
        parent_handle: Handle<NTreeNode<TexturePackerEntry>>,
        child_handle: Handle<NTreeNode<TexturePackerEntry>>,
    ) -> bool {
        !self.is_left_child(parent_handle, child_handle)
    }

    fn insert_at(
        &mut self,
        width: u32,
        height: u32,
        handle: Handle<NTreeNode<TexturePackerEntry>>,
    ) -> Option<Handle<NTreeNode<TexturePackerEntry>>> {
        if !self.is_leaf(handle) {
            // try inserting under left child
            let left_child_handle = self
                .ntree
                .get(handle)
                .first_child
                .expect("left child handle");
            let new_handle = self.insert_at(width, height, left_child_handle);
            if new_handle.is_some() {
                return new_handle;
            }

            // no room, insert under right child
            let right_child_handle = self
                .ntree
                .get(left_child_handle)
                .next_sibling
                .expect("right child handle");
            return self.insert_at(width, height, right_child_handle);
        }

        // there is already a glpyh here
        if self.ntree.get(handle).value.in_use {
            return None;
        }

        let cache_slot_width = self.ntree.get(handle).value.w;
        let cache_slot_height = self.ntree.get(handle).value.h;

        if width > cache_slot_width || height > cache_slot_height {
            // if this node's box is too small, return
            return None;
        }

        if width == cache_slot_width && height == cache_slot_height {
            // if we're just right, accept
            self.ntree.get_mut(handle).value.in_use = true;
            return Some(handle);
        }

        // otherwise, gotta split this node and create some kids decide which way to split
        let dw = cache_slot_width - width;
        let dh = cache_slot_height - height;

        let left_child_handle = if dw > dh {
            // split along x

            let left_child = {
                let entry = &self.ntree.get(handle).value;
                TexturePackerEntry {
                    w: width,
                    h: cache_slot_height,
                    in_use: false,
                    ..*entry
                }
            };
            let left_child_handle = self
                .ntree
                .insert_child_after(Some(handle), None, left_child);
            assert!(self.is_left_child(handle, left_child_handle));

            let right_child = {
                let entry = &self.ntree.get(handle).value;
                TexturePackerEntry {
                    x: entry.x + width,
                    w: dw,
                    h: cache_slot_height,
                    in_use: false,
                    ..*entry
                }
            };
            let right_child_handle =
                self.ntree
                    .insert_child_after(Some(handle), Some(left_child_handle), right_child);
            assert!(self.is_right_child(handle, right_child_handle));

            assert!(
                self.ntree.get(left_child_handle).parent
                    == self.ntree.get(right_child_handle).parent
            );
            assert!(self.ntree.get(left_child_handle).parent == Some(handle));

            left_child_handle
        } else {
            // split along y

            let left_child = {
                let entry = &self.ntree.get(handle).value;
                TexturePackerEntry {
                    w: cache_slot_width,
                    h: height,
                    in_use: false,
                    ..*entry
                }
            };
            let left_child_handle = self
                .ntree
                .insert_child_after(Some(handle), None, left_child);
            assert!(self.is_left_child(handle, left_child_handle));

            let right_child = {
                let entry = &self.ntree.get(handle).value;
                TexturePackerEntry {
                    y: entry.y + height,
                    w: cache_slot_width,
                    h: dh,
                    in_use: false,
                    ..*entry
                }
            };
            let right_child_handle =
                self.ntree
                    .insert_child_after(Some(handle), Some(left_child_handle), right_child);
            assert!(self.is_right_child(handle, right_child_handle));

            assert!(
                self.ntree.get(left_child_handle).parent
                    == self.ntree.get(right_child_handle).parent
            );
            assert!(self.ntree.get(left_child_handle).parent == Some(handle));

            left_child_handle
        };

        // insert into first child we created
        self.insert_at(width, height, left_child_handle)
    }

    /// NOTE: returned handle may be dangling meaning that there's not enough space to accomodate
    /// rect.
    pub fn insert(
        &mut self,
        width: u32,
        height: u32,
    ) -> Option<Handle<NTreeNode<TexturePackerEntry>>> {
        self.insert_at(width, height, self.ntree.root())
    }

    pub fn get(&self, handle: Handle<NTreeNode<TexturePackerEntry>>) -> &TexturePackerEntry {
        &self.ntree.get(handle).value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_exact_fit() {
        let mut packer = TexturePacker::default();

        let maybe_handle = packer.insert(DEFAULT_TEXTURE_WIDTH, DEFAULT_TEXTURE_HEIGHT);
        assert!(maybe_handle.is_some());

        let handle = maybe_handle.unwrap();
        let entry = &packer.ntree.get(handle).value;
        assert!(entry.in_use);
        assert_eq!(entry.x, 0);
        assert_eq!(entry.y, 0);
        assert_eq!(entry.w, DEFAULT_TEXTURE_WIDTH);
        assert_eq!(entry.h, DEFAULT_TEXTURE_HEIGHT);
    }

    #[test]
    fn test_insert_too_large() {
        let mut packer = TexturePacker::default();

        let maybe_handle = packer.insert(DEFAULT_TEXTURE_WIDTH, DEFAULT_TEXTURE_HEIGHT + 1);
        assert!(maybe_handle.is_none());
    }

    #[test]
    fn test_horizontal_split() {
        let mut packer = TexturePacker::default();

        // create a rectangle that will cause a horizontal split (width difference > height
        // difference)
        let maybe_handle1 = packer.insert(400, DEFAULT_TEXTURE_HEIGHT);
        assert!(maybe_handle1.is_some());

        // the root should now have two children
        let root = packer.ntree.root();
        assert!(!packer.is_leaf(root));

        // try inserting another rectangle in the remaining space
        let maybe_handle2 = packer.insert(400, DEFAULT_TEXTURE_HEIGHT);
        assert!(maybe_handle2.is_some());

        assert!(maybe_handle1 != maybe_handle2);
    }

    #[test]
    fn test_vertical_split() {
        let mut packer = TexturePacker::default();

        // create a rectangle that will cause a vertical split (height difference > width
        // difference)
        let maybe_handle1 = packer.insert(DEFAULT_TEXTURE_WIDTH, 400);
        assert!(maybe_handle1.is_some());

        // the root should now have two children
        let root = packer.ntree.root();
        assert!(!packer.is_leaf(root));

        // try inserting another rectangle in the remaining space
        let maybe_handle2 = packer.insert(DEFAULT_TEXTURE_WIDTH, 400);
        assert!(maybe_handle2.is_some());

        assert!(maybe_handle1 != maybe_handle2);
    }
}
