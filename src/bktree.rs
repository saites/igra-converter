use std::cmp::Ordering;
use std::fmt;
use std::ops::Sub;

pub trait Metric<Rhs = Self> {
    type Output: Ord + Copy + Sub;

    fn dist(&self, x: &Rhs) -> Self::Output;
}

pub struct BKTree<T, O>
    where
        O: Ord + Copy + Sub<Output=O>,
        T: Metric<Output=O>,
{
    root: Option<BKTreeNode<T, O>>,
    /// Number of entries in the tree.
    size: usize,
}

impl<T, O> fmt::Debug for BKTree<T, O>
    where
        O: Ord + Copy + Sub<Output=O> + fmt::Debug,
        T: Metric<Output=O> + fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.size == 0 {
            write!(f, "Empty BKTree")
        } else if f.alternate() {
            f.debug_struct("BKTree")
                .field("size", &self.size)
                .field("root", &self.root)
                .finish()
        } else {
            f.debug_struct("BKTree")
                .field("size", &self.size)
                .finish_non_exhaustive()
        }
    }
}

impl<T, O> BKTree<T, O>
    where
        O: Ord + Copy + Sub<Output=O>,
        T: Metric<Output=O>,
{
    pub fn new() -> Self {
        BKTree {
            root: None,
            size: 0,
        }
    }

    pub fn insert(&mut self, item: T) {
        if let Some(ref mut r) = self.root {
            r.insert(item);
        } else {
            self.root = Some(BKTreeNode::new(item));
        }
        self.size += 1;
    }

    /// Find elements within a certain distance of the given element.
    pub fn find<S>(&self, item: &S, max_dist: O) -> Vec<(O, &T)>
    where
        S: Metric<T, Output=O>,
    {
        self.find_by(max_dist, |x| item.dist(x))
    }

    /// Find elements within a certain distance of the given element,
    /// using the given `dist` function rather than its metric.
    ///
    /// It's assumed that the distance function used is compatible with the metric used to build the tree;
    /// the point of this function is to allow searching the tree using a type other than the original
    /// without needing to reimplement the metric.
    pub fn find_by<F>(&self, max_dist: O, dist: F) -> Vec<(O, &T)>
        where
            F: Fn(&T) -> O
    {
        if let Some(r) = &self.root {
            let (cnt, v) = r.find_by(max_dist, dist);
            log::debug!(
                "Processed {cnt} of {total} nodes and found {v_len} items.",
                total=self.size, v_len=v.len()
            );
            return v;
        } else {
            vec![]
        }
    }

    pub fn find_closest<F>(&self, max_dist: O, dist: F) -> Option<(O, &T)>
        where
            F: Fn(&T) -> O
    {
        if let [first, ..] = self.find_by(max_dist, dist)[..] {
            Some(first)
        } else {
            None
        }
    }
}


/// An internal node which stores a value and a list of children and their associated distances.
///
/// For `Some((dist, child)) = self.children[i]`, every descendant of `child` is `dist` from `self`.
#[derive(Debug)]
struct BKTreeNode<T, O>
    where
        O: Ord + Copy + Sub<Output=O>,
        T: Metric<Output=O>,
{
    value: T,
    children: Option<Vec<(O, BKTreeNode<T, O>)>>,
}

/// A node which enqueued for processing during a search through the tree.
///
/// The next few blocks implement equality/comparisons for this type
/// so that it can be sorted later.
struct ProcNode<'a, T, O>
    where
        O: Ord + Copy + Sub<Output=O>,
        T: Metric<Output=O>,
{
    u: &'a BKTreeNode<T, O>,
    dist_wu: O,
    id: usize,
}

impl<'a, T, O> Ord for ProcNode<'a, T, O>
    where
        O: Ord + Copy + Sub<Output=O>,
        T: Metric<Output=O>,
{
    fn cmp(&self, other: &Self) -> Ordering {
        other.dist_wu.cmp(&self.dist_wu).then_with(|| self.id.cmp(&other.id))
    }
}

impl<'a, T, O> PartialOrd<Self> for ProcNode<'a, T, O>
    where
        O: Ord + Copy + Sub<Output=O>,
        T: Metric<Output=O>,
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<'a, T, O> Eq for ProcNode<'a, T, O>
    where
        O: Ord + Copy + Sub<Output=O>,
        T: Metric<Output=O>,
{}

impl<'a, T, O> PartialEq<Self> for ProcNode<'a, T, O>
    where
        O: Ord + Copy + Sub<Output=O>,
        T: Metric<Output=O>,
{
    fn eq(&self, other: &Self) -> bool {
        self.dist_wu == other.dist_wu && self.id == other.id
    }
}

/// This is the actual tree implementation.
impl<T, O> BKTreeNode<T, O>
    where
        O: Ord + Copy + Copy + Sub<Output = O>,
        T: Metric<Output=O>
{
    /// Create a new BKTree rooted at T.
    fn new(root: T) -> Self {
        BKTreeNode { value: root, children: None }
    }

    /// Insert an item into the tree.
    ///
    /// It works by determining its distance from the given item
    /// and searching through `self.children` for a child at the same distance.
    /// If it finds one, it calls its `insert` method, recursively traversing the tree
    /// until it finds a node that does not yet have a child of the same distance as its distance to `item`.
    /// There, it creates a new leaf node and adds it to the tree.
    fn insert(&mut self, item: T) {
        let k = self.value.dist(&item);
        // If Metric should be a proper metric (not a pseudometric),
        // (i.e., enforce the metric property that dist(x, y) == 0 <=> x == y)
        // uncomment the line below:
        // if k == 0 { return; }

        if let Some(ref mut c) = self.children {
            match c.iter_mut().find_map(|(duv, v)| if *duv == k { Some(v) } else { None }) {
                None => { c.push((k, Self::new(item))); }
                Some(v) => { v.insert(item); }
            }
        } else {
            self.children = Some(vec![(k, Self::new(item))]);
        }
    }

    /// Find the closest elements that are no more than max_dist from the given item.
    /// Returns (number of nodes processed, Vec<(distance to &T, &T)>).
    fn find_by<F>(&self, max_dist: O, dist: F) -> (usize, Vec<(O, &T)>)
        where
            F: Fn(&T) -> O
    {
        let mut s = Vec::new();
        let mut r = Vec::new();

        let d_wu = dist(&self.value);
        s.push(ProcNode { u: self, dist_wu: d_wu, id: 0 });

        let mut cnt = 0;
        while let Some(ProcNode { u, dist_wu, id: _ }) = s.pop() {
            cnt += 1;

            if dist_wu <= max_dist {
                r.push((dist_wu, &u.value));
            }

            // Add children that live on a hypersphere that intersects our tolerance.
            if let Some(c) = &u.children {
                for (dist_uv, v) in c {
                    let diff = if dist_wu < *dist_uv {
                        dist_uv.sub(dist_wu)
                    } else {
                        dist_wu.sub(*dist_uv)
                    };
                    if diff <= max_dist {
                        s.push(ProcNode { u: v, dist_wu: dist(&v.value), id: cnt });
                    }
                }
            }
        }

        r.sort_by(|(d0, _), (d1, _)| d0.cmp(d1));
        (cnt, r)
    }
}