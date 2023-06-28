use std::collections::{BinaryHeap, VecDeque};

pub trait Metric {
    type Item;

    fn dist(&self, x: &Self::Item) -> usize;
}


pub struct BKTree<T: Metric> {
    value: T,
    children: Option<Vec<(usize, BKTree<T>)>>,
}

impl<T: Metric<Item = T>> BKTree<T> {
    /// Create a new BKTree rooted at T.
    pub fn new(root: T) -> Self {
        BKTree{ value: root, children: None }
    }

    /// Insert an item into the tree.
    pub fn insert(&mut self, item: T) {
        let k = self.value.dist(&item);
        if k == 0 {
            return
        }

        if let Some(ref mut c) = self.children {
            match c.iter_mut().find_map(|(duv, v)| if *duv == k { Some(v) } else { None }) {
                None => { c.push((k, BKTree::new(item))); }
                Some(v) => { v.insert(item); }
            }
        } else {
            self.children = Some(vec![(k, BKTree::new(item))]);
        }
    }

    /// Find the closest element to the given element.
    ///
    /// If the tree only contains `item`, this returns it.
    /// Otherwise, it'll be the closest non-identity element.
    /// Note that by the properties of the Metric,
    /// two items are equal if and only if their distance is 0.
    pub fn find_closest(&self, item: &T) -> &T {
        // let mut s = BinaryHeap::new();
        let mut s = VecDeque::new();
        s.push_back(self);

        let mut w_best = self;
        let mut d_best = item.dist(&self.value);
        while let Some(u) = s.pop_front() {
            let d_u = item.dist(&u.value);
            // This assumes that 0 is only true for an exact match,
            // which we don't care about.
            if d_best == 0 || d_u > 0 && d_u < d_best {
                w_best = u;
                d_best = d_u;
            }

            if let Some(c) = &u.children {
                for (d_uv, v) in c {
                    if d_uv.abs_diff(d_u) < d_best {
                        s.push_back(v);
                    }
                }
            }
        }

        &w_best.value
    }
}