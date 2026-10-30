//! A binary min-heap that performs exactly the comparisons CPython's
//! `heapq.heappush` and `heapq.heappop` perform.
//!
//! RP2 ranks lots with a comparison that is not transitive (13-place decimal
//! equality), so the order of comparisons decides which lot wins a near-tie.
//! This heap reproduces CPython's sift procedures step for step, and its
//! comparator may fail, as a Python `__lt__` may raise.

/// A heap ordered by a fallible strict less-than.
#[derive(Clone, Debug, Default)]
pub struct PyHeap<T> {
    items: Vec<T>,
}

impl<T> PyHeap<T> {
    /// An empty heap.
    pub fn new() -> Self {
        PyHeap { items: Vec::new() }
    }

    /// `heapq.heappush`: append, then sift the new item toward the root.
    pub fn push<E>(
        &mut self,
        item: T,
        less: &mut impl FnMut(&T, &T) -> Result<bool, E>,
    ) -> Result<(), E> {
        self.items.push(item);
        let last = self.items.len() - 1;
        self.sift_down(0, last, less)
    }

    /// `heapq.heappop`: remove the last item; if items remain, return the
    /// root and sift the removed item down from the root.
    pub fn pop<E>(
        &mut self,
        less: &mut impl FnMut(&T, &T) -> Result<bool, E>,
    ) -> Result<Option<T>, E> {
        let Some(last) = self.items.pop() else {
            return Ok(None);
        };
        if self.items.is_empty() {
            return Ok(Some(last));
        }
        let root = std::mem::replace(&mut self.items[0], last);
        self.sift_up(0, less)?;
        Ok(Some(root))
    }

    /// CPython's `_siftdown`: move the item at `pos` toward `start` while it
    /// is less than its parent.
    fn sift_down<E>(
        &mut self,
        start: usize,
        mut pos: usize,
        less: &mut impl FnMut(&T, &T) -> Result<bool, E>,
    ) -> Result<(), E> {
        while pos > start {
            let parent = (pos - 1) >> 1;
            if less(&self.items[pos], &self.items[parent])? {
                self.items.swap(pos, parent);
                pos = parent;
            } else {
                break;
            }
        }
        Ok(())
    }

    /// CPython's `_siftup`: walk the item at `pos` down to a leaf, always
    /// following the right child unless the left one is less, then sift it
    /// back up with [`Self::sift_down`].
    fn sift_up<E>(
        &mut self,
        mut pos: usize,
        less: &mut impl FnMut(&T, &T) -> Result<bool, E>,
    ) -> Result<(), E> {
        let end = self.items.len();
        let start = pos;
        let mut child = 2 * pos + 1;
        while child < end {
            let right = child + 1;
            if right < end && !less(&self.items[child], &self.items[right])? {
                child = right;
            }
            // Moving the child up and the item down is the swap CPython's
            // hole-based loop performs; the item compares only at the end.
            self.items.swap(pos, child);
            pos = child;
            child = 2 * pos + 1;
        }
        self.sift_down(start, pos, less)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn less(a: &i32, b: &i32) -> Result<bool, ()> {
        Ok(a < b)
    }

    #[test]
    fn pops_in_order_with_a_total_order() {
        let mut heap = PyHeap::new();
        for value in [5, 3, 8, 1, 9, 2, 7, 3] {
            heap.push(value, &mut less).unwrap();
        }
        let mut popped = Vec::new();
        while let Some(value) = heap.pop(&mut less).unwrap() {
            popped.push(value);
        }
        assert_eq!(popped, vec![1, 2, 3, 3, 5, 7, 8, 9]);
        assert!(heap.items.is_empty());
    }

    #[test]
    fn layout_matches_cpython() {
        // heapq.heappush over [5, 3, 8, 1, 9, 2] gives [1, 3, 2, 5, 9, 8];
        // one heappop then leaves [2, 3, 8, 5, 9].
        let mut heap = PyHeap::new();
        for value in [5, 3, 8, 1, 9, 2] {
            heap.push(value, &mut less).unwrap();
        }
        assert_eq!(heap.items, vec![1, 3, 2, 5, 9, 8]);
        assert_eq!(heap.pop(&mut less).unwrap(), Some(1));
        assert_eq!(heap.items, vec![2, 3, 8, 5, 9]);
    }
}
