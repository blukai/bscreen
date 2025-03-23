use crate::genvec::{GenVec, Handle};

/// be sane, do not mutate handles from the outside.
#[derive(Debug)]
pub struct NTreeNode<T> {
    pub value: T,
    pub parent: Option<Handle<Self>>,
    pub first_child: Option<Handle<Self>>,
    pub prev_sibling: Option<Handle<Self>>,
    pub next_sibling: Option<Handle<Self>>,
}

impl<T> NTreeNode<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            parent: None,
            first_child: None,
            prev_sibling: None,
            next_sibling: None,
        }
    }
}

/// an n-way tree (also known as a multiway tree).
#[derive(Debug, Default)]
pub struct NTree<T> {
    nodes: GenVec<NTreeNode<T>>,
    root: Handle<NTreeNode<T>>,
}

impl<T> NTree<T> {
    pub fn new(value: T) -> Self {
        let mut this = Self {
            nodes: GenVec::default(),
            root: Handle::DANGLING,
        };
        this.root = this.new_orphan(value);
        this
    }

    pub fn unlink(&mut self, handle: Handle<NTreeNode<T>>) {
        assert!(!handle.is_dangling());

        let (ticket, mut node) = self.nodes.take(handle);

        // if we're the first guy, reset the head otherwise, make our previous node's next pointer
        // = our next
        if let Some(prev_sibling_node) = node.prev_sibling.map(|h| self.nodes.get_mut(h)) {
            prev_sibling_node.next_sibling = node.next_sibling;
        } else {
            if let Some(parent_node) = node.parent.map(|h| self.nodes.get_mut(h)) {
                parent_node.first_child = node.next_sibling;
            } else if self.root == handle {
                self.root = node.next_sibling.expect("next sibling");
            }
        }

        // if we're the last guy, reset the tail otherwise, make our next node's prev pointer = our
        // prev
        if let Some(next_sibling_node) = node.next_sibling.map(|h| self.nodes.get_mut(h)) {
            next_sibling_node.prev_sibling = node.prev_sibling;
        }

        // unlink everything except children
        node.parent = None;
        node.prev_sibling = None;
        node.next_sibling = None;

        self.nodes.put_back(ticket, node);
    }

    pub fn remove(&mut self, handle: Handle<NTreeNode<T>>) -> T {
        self.unlink(handle);
        self.nodes.remove(handle).value
    }

    /// links a node after a particular node
    fn link_child_after(
        &mut self,
        parent: Option<Handle<NTreeNode<T>>>,
        after: Option<Handle<NTreeNode<T>>>,
        child: Handle<NTreeNode<T>>,
    ) {
        assert!(!child.is_dangling());

        self.unlink(child);

        let (ticket, mut node) = self.nodes.take(child);

        node.parent = parent;
        node.prev_sibling = after;
        if let Some(prev_sibling_node) = node.prev_sibling.map(|h| self.nodes.get_mut(h)) {
            node.next_sibling = prev_sibling_node.next_sibling;
            prev_sibling_node.next_sibling = Some(child);
        } else {
            if let Some(parent_node) = node.parent.map(|h| self.nodes.get_mut(h)) {
                node.next_sibling = parent_node.first_child;
                parent_node.first_child = Some(child);
            } else {
                node.next_sibling = Some(self.root);
                if let Some(next_sibling_node) = node.next_sibling.map(|h| self.nodes.get_mut(h)) {
                    next_sibling_node.prev_sibling = Some(child);
                }
                self.root = child;
            }
        }

        // TODO: can this be somehow combined with the similar thing above?
        if let Some(next_sibling_node) = node.next_sibling.map(|h| self.nodes.get_mut(h)) {
            next_sibling_node.prev_sibling = Some(child);
        }

        self.nodes.put_back(ticket, node);
    }

    /// links a node before a particular node
    fn link_child_before(
        &mut self,
        parent: Option<Handle<NTreeNode<T>>>,
        before: Option<Handle<NTreeNode<T>>>,
        child: Handle<NTreeNode<T>>,
    ) {
        assert!(!child.is_dangling());

        if let Some(before_node) = before.map(|h| self.nodes.get(h)) {
            self.link_child_after(parent, before_node.prev_sibling, child);
            return;
        }

        let mut after_handle = if let Some(parent_node) = parent.map(|h| self.nodes.get(h)) {
            parent_node.first_child
        } else {
            Some(self.root)
        };
        if after_handle.is_none() {
            self.link_child_after(parent, after_handle, child);
            return;
        }

        let mut next_handle = after_handle.and_then(|h| self.nodes.get(h).next_sibling);
        while let Some(next_node) = next_handle.map(|h| self.nodes.get(h)) {
            after_handle = next_handle;
            next_handle = next_node.next_sibling;
        }

        self.link_child_after(parent, after_handle, child);
    }

    fn new_orphan(&mut self, value: T) -> Handle<NTreeNode<T>> {
        self.nodes.insert(NTreeNode::new(value))
    }

    pub fn insert_child_after(
        &mut self,
        parent: Option<Handle<NTreeNode<T>>>,
        after: Option<Handle<NTreeNode<T>>>,
        value: T,
    ) -> Handle<NTreeNode<T>> {
        let handle = self.new_orphan(value);
        self.link_child_after(parent, after, handle);
        handle
    }

    pub fn insert_child_before(
        &mut self,
        parent: Option<Handle<NTreeNode<T>>>,
        before: Option<Handle<NTreeNode<T>>>,
        value: T,
    ) -> Handle<NTreeNode<T>> {
        let handle = self.new_orphan(value);
        self.link_child_before(parent, before, handle);
        handle
    }

    pub fn root(&self) -> Handle<NTreeNode<T>> {
        self.root
    }

    // gen vec delegates

    #[inline]
    pub fn try_get(&self, handle: Handle<NTreeNode<T>>) -> Option<&NTreeNode<T>> {
        self.nodes.try_get(handle)
    }

    #[inline]
    pub fn get(&self, handle: Handle<NTreeNode<T>>) -> &NTreeNode<T> {
        self.nodes.get(handle)
    }

    #[inline]
    pub fn try_get_mut(&mut self, handle: Handle<NTreeNode<T>>) -> Option<&mut NTreeNode<T>> {
        self.nodes.try_get_mut(handle)
    }

    #[inline]
    pub fn get_mut(&mut self, handle: Handle<NTreeNode<T>>) -> &mut NTreeNode<T> {
        self.nodes.get_mut(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_child_after() {
        let mut tree = NTree::new(0);
        let root = tree.root();

        // insert first child
        let child1 = tree.insert_child_after(Some(root), None, 1);
        assert_eq!(tree.get(child1).parent, Some(root));
        assert_eq!(tree.get(root).first_child, Some(child1));

        // insert second child after first
        let child2 = tree.insert_child_after(Some(root), Some(child1), 2);
        assert_eq!(tree.get(child2).parent, Some(root));
        assert_eq!(tree.get(child1).next_sibling, Some(child2));
        assert_eq!(tree.get(child2).prev_sibling, Some(child1));

        // insert third child after second
        let child3 = tree.insert_child_after(Some(root), Some(child2), 3);
        assert_eq!(tree.get(child3).parent, Some(root));
        assert_eq!(tree.get(child2).next_sibling, Some(child3));
        assert_eq!(tree.get(child3).prev_sibling, Some(child2));
        assert!(tree.get(child3).next_sibling.is_none());
    }

    #[test]
    fn test_insert_child_before() {
        let mut tree = NTree::new(0);
        let root = tree.root();

        // insert first child
        let child1 = tree.insert_child_before(Some(root), None, 1);
        assert_eq!(tree.get(child1).parent, Some(root));
        assert_eq!(tree.get(root).first_child, Some(child1));

        // insert second child before first
        let child2 = tree.insert_child_before(Some(root), Some(child1), 2);
        assert_eq!(tree.get(child2).parent, Some(root));
        assert_eq!(tree.get(root).first_child, Some(child2));
        assert_eq!(tree.get(child2).next_sibling, Some(child1));
        assert_eq!(tree.get(child1).prev_sibling, Some(child2));

        // insert third child before second
        let child3 = tree.insert_child_before(Some(root), Some(child2), 3);
        assert_eq!(tree.get(child3).parent, Some(root));
        assert_eq!(tree.get(root).first_child, Some(child3));
        assert_eq!(tree.get(child3).next_sibling, Some(child2));
        assert_eq!(tree.get(child2).prev_sibling, Some(child3));
    }

    #[test]
    fn test_nested_children() {
        let mut tree = NTree::new(0);
        let root = tree.root();

        let child1 = tree.insert_child_after(Some(root), None, 1);
        let grandchild1 = tree.insert_child_after(Some(child1), None, 2);
        let grandchild2 = tree.insert_child_after(Some(child1), Some(grandchild1), 3);

        assert_eq!(tree.get(grandchild1).parent, Some(child1));
        assert_eq!(tree.get(grandchild2).parent, Some(child1));
        assert_eq!(tree.get(child1).first_child, Some(grandchild1));
        assert_eq!(tree.get(grandchild1).next_sibling, Some(grandchild2));
    }

    #[test]
    fn test_unlink_node() {
        let mut tree = NTree::new(0);
        let root = tree.root();

        let child1 = tree.insert_child_after(Some(root), None, 1);
        let child2 = tree.insert_child_after(Some(root), Some(child1), 2);
        let child3 = tree.insert_child_after(Some(root), Some(child2), 3);

        tree.unlink(child2);
        // check that child1 now points to child3
        assert_eq!(tree.get(child1).next_sibling, Some(child3));
        assert_eq!(tree.get(child3).prev_sibling, Some(child1));
        // check that child2 is unlinked but still exists
        assert!(tree.get(child2).parent.is_none());
        assert!(tree.get(child2).prev_sibling.is_none());
        assert!(tree.get(child2).next_sibling.is_none());

        tree.unlink(child1);
        // check that root now points to child3 as first child
        assert_eq!(tree.get(root).first_child, Some(child3));
        assert!(tree.get(child3).prev_sibling.is_none());

        tree.unlink(child3);
        // check that root has no children
        assert!(tree.get(root).first_child.is_none());
    }
}
