//! Split - tiling tree of panes within a tab

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SplitDir {
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SplitNode {
    Leaf(usize),
    Split {
        dir: SplitDir,
        children: Vec<SplitNode>,
        ratio: f32,
    },
}

impl SplitNode {
    pub fn insert_leaf(&mut self, pane_id: usize, dir: SplitDir, parent: usize, ratio: f32) {
        let tree = std::mem::replace(self, SplitNode::Leaf(parent));
        *self = insert_into(tree, pane_id, dir, parent, ratio);
    }

    pub fn remove_leaf(&mut self, pane_id: usize) {
        let tree = std::mem::replace(self, SplitNode::Leaf(pane_id));
        if let Some(tree) = remove_from(tree, pane_id) {
            *self = tree;
        }
    }

    pub fn leaves(&self) -> Vec<usize> {
        match self {
            SplitNode::Leaf(id) => vec![*id],
            SplitNode::Split { children, .. } => {
                children.iter().flat_map(SplitNode::leaves).collect()
            }
        }
    }

    /// Rebuild a tree from a flat pane-id list (pane order preserved, left to right).
    /// Empty list -> Leaf(0) so the default pane still renders.
    pub fn from_ids(ids: &[usize]) -> Self {
        fn build(ids: &[usize], depth: usize) -> SplitNode {
            match ids.len() {
                0 => SplitNode::Leaf(0),
                1 => SplitNode::Leaf(ids[0]),
                n => {
                    let mid = n / 2;
                    let dir = if depth % 2 == 0 {
                        SplitDir::Vertical
                    } else {
                        SplitDir::Horizontal
                    };
                    SplitNode::Split {
                        dir,
                        ratio: 0.5,
                        children: vec![
                            build(&ids[..mid], depth + 1),
                            build(&ids[mid..], depth + 1),
                        ],
                    }
                }
            }
        }
        build(ids, 0)
    }

    pub fn compute_rects(&self) -> HashMap<usize, (f32, f32, f32, f32)> {
        let mut rects = HashMap::new();
        self.assign_rects(0.0, 0.0, 1.0, 1.0, &mut rects);
        rects
    }

    fn assign_rects(
        &self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        out: &mut HashMap<usize, (f32, f32, f32, f32)>,
    ) {
        match self {
            SplitNode::Leaf(id) => {
                out.insert(*id, (x, y, w, h));
            }
            SplitNode::Split {
                dir,
                children,
                ratio,
            } => {
                let n = children.len().max(1);
                match dir {
                    SplitDir::Vertical => {
                        let first_w = w * ratio;
                        let rest_w = if n > 1 {
                            (w - first_w) / (n - 1) as f32
                        } else {
                            0.0
                        };
                        let mut cx = x;
                        for (i, child) in children.iter().enumerate() {
                            let cw = if i == 0 { first_w } else { rest_w };
                            child.assign_rects(cx, y, cw, h, out);
                            cx += cw;
                        }
                    }
                    SplitDir::Horizontal => {
                        let first_h = h * ratio;
                        let rest_h = if n > 1 {
                            (h - first_h) / (n - 1) as f32
                        } else {
                            0.0
                        };
                        let mut cy = y;
                        for (i, child) in children.iter().enumerate() {
                            let ch = if i == 0 { first_h } else { rest_h };
                            child.assign_rects(x, cy, w, ch, out);
                            cy += ch;
                        }
                    }
                }
            }
        }
    }

    /// Divider lines as (dir, normalized boundary position, first-leaf target).
    /// Boundary position is in the normalized 0..1 content space (x for Vertical,
    /// y for Horizontal), matching compute_rects. DFS order, outermost first.
    pub fn dividers(&self) -> Vec<(SplitDir, f32, usize)> {
        let mut out = Vec::new();
        self.collect_dividers(0.0, 0.0, 1.0, 1.0, &mut out);
        out
    }

    fn collect_dividers(
        &self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        out: &mut Vec<(SplitDir, f32, usize)>,
    ) {
        match self {
            SplitNode::Leaf(_) => {}
            SplitNode::Split {
                dir,
                children,
                ratio,
            } => {
                let n = children.len().max(1);
                match dir {
                    SplitDir::Vertical => {
                        let first_w = w * ratio;
                        let target = children[0].leaves()[0];
                        out.push((SplitDir::Vertical, x + first_w, target));
                        let rest_w = if n > 1 {
                            (w - first_w) / (n - 1) as f32
                        } else {
                            0.0
                        };
                        let mut cx = x;
                        for (i, child) in children.iter().enumerate() {
                            let cw = if i == 0 { first_w } else { rest_w };
                            child.collect_dividers(cx, y, cw, h, out);
                            cx += cw;
                        }
                    }
                    SplitDir::Horizontal => {
                        let first_h = h * ratio;
                        let target = children[0].leaves()[0];
                        out.push((SplitDir::Horizontal, y + first_h, target));
                        let rest_h = if n > 1 {
                            (h - first_h) / (n - 1) as f32
                        } else {
                            0.0
                        };
                        let mut cy = y;
                        for (i, child) in children.iter().enumerate() {
                            let ch = if i == 0 { first_h } else { rest_h };
                            child.collect_dividers(x, cy, w, ch, out);
                            cy += ch;
                        }
                    }
                }
            }
        }
    }

    /// Adjust `ratio` of the split whose divider sits at `boundary` and whose
    /// first-leaf is `target`. `delta` is the normalized move amount (positive
    /// grows the first child). Clamps ratio to 0.15..0.85.
    pub fn resize_leaf(&mut self, target: usize, boundary: f32, delta: f32) -> bool {
        self.resize_walk(0.0, 0.0, 1.0, 1.0, target, boundary, delta)
    }

    fn resize_walk(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        target: usize,
        boundary: f32,
        delta: f32,
    ) -> bool {
        match self {
            SplitNode::Leaf(_) => false,
            SplitNode::Split {
                dir,
                children,
                ratio,
            } => {
                let n = children.len().max(1);
                let r = *ratio;
                let (first_len, rest_len, divider_pos) = match dir {
                    SplitDir::Vertical => (
                        w * r,
                        if n > 1 {
                            (w - w * r) / (n - 1) as f32
                        } else {
                            0.0
                        },
                        x + w * r,
                    ),
                    SplitDir::Horizontal => (
                        h * r,
                        if n > 1 {
                            (h - h * r) / (n - 1) as f32
                        } else {
                            0.0
                        },
                        y + h * r,
                    ),
                };
                if (divider_pos - boundary).abs() < 0.001 && children[0].leaves()[0] == target {
                    *ratio = (*ratio + delta).clamp(0.15, 0.85);
                    return true;
                }
                let mut cx = x;
                let mut cy = y;
                for (i, child) in children.iter_mut().enumerate() {
                    let (cw, ch) = match dir {
                        SplitDir::Vertical => {
                            let cw = if i == 0 { first_len } else { rest_len };
                            (cw, h)
                        }
                        SplitDir::Horizontal => {
                            let ch = if i == 0 { first_len } else { rest_len };
                            (w, ch)
                        }
                    };
                    if child.resize_walk(cx, cy, cw, ch, target, boundary, delta) {
                        return true;
                    }
                    match dir {
                        SplitDir::Vertical => cx += cw,
                        SplitDir::Horizontal => cy += ch,
                    }
                }
                false
            }
        }
    }
}

fn insert_into(
    tree: SplitNode,
    pane_id: usize,
    dir: SplitDir,
    parent: usize,
    ratio: f32,
) -> SplitNode {
    match tree {
        SplitNode::Leaf(id) if id == parent => SplitNode::Split {
            dir,
            children: vec![SplitNode::Leaf(id), SplitNode::Leaf(pane_id)],
            ratio,
        },
        SplitNode::Leaf(_) => tree,
        SplitNode::Split {
            dir,
            children,
            ratio: r,
        } => {
            let children = children
                .into_iter()
                .map(|c| insert_into(c, pane_id, dir, parent, ratio))
                .collect();
            SplitNode::Split {
                dir,
                children,
                ratio: r,
            }
        }
    }
}

fn remove_from(tree: SplitNode, pane_id: usize) -> Option<SplitNode> {
    match tree {
        SplitNode::Leaf(id) => {
            if id == pane_id {
                None
            } else {
                Some(SplitNode::Leaf(id))
            }
        }
        SplitNode::Split {
            dir,
            children,
            ratio,
        } => {
            let children: Vec<SplitNode> = children
                .into_iter()
                .filter_map(|c| remove_from(c, pane_id))
                .collect();
            if children.is_empty() {
                None
            } else if children.len() == 1 {
                Some(children.into_iter().next().unwrap())
            } else {
                Some(SplitNode::Split {
                    dir,
                    children,
                    ratio,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_splits_parent_leaf() {
        let mut root = SplitNode::Leaf(1);
        root.insert_leaf(2, SplitDir::Vertical, 1, 0.5);
        assert_eq!(root.leaves(), vec![1, 2]);
        match root {
            SplitNode::Split {
                dir,
                children,
                ratio,
            } => {
                assert_eq!(dir, SplitDir::Vertical);
                assert_eq!(children.len(), 2);
                assert_eq!(ratio, 0.5);
            }
            _ => panic!("expected split"),
        }
    }

    #[test]
    fn nested_insert_keeps_order() {
        let mut root = SplitNode::Leaf(1);
        root.insert_leaf(2, SplitDir::Vertical, 1, 0.5);
        root.insert_leaf(3, SplitDir::Horizontal, 2, 0.5);
        assert_eq!(root.leaves(), vec![1, 2, 3]);
    }

    #[test]
    fn remove_collapses_single_child() {
        let mut root = SplitNode::Leaf(1);
        root.insert_leaf(2, SplitDir::Vertical, 1, 0.5);
        root.insert_leaf(3, SplitDir::Horizontal, 2, 0.5);
        assert_eq!(root.leaves(), vec![1, 2, 3]);
        root.remove_leaf(3);
        assert_eq!(root.leaves(), vec![1, 2]);
        root.remove_leaf(2);
        assert_eq!(root.leaves(), vec![1]);
    }

    #[test]
    fn from_ids_preserves_order_and_defaults() {
        assert_eq!(
            SplitNode::from_ids(&[1, 2, 3, 4]).leaves(),
            vec![1, 2, 3, 4]
        );
        assert_eq!(SplitNode::from_ids(&[7]).leaves(), vec![7]);
        assert_eq!(SplitNode::from_ids(&[]).leaves(), vec![0]);
    }

    #[test]
    fn vertical_rects_side_by_side() {
        let mut root = SplitNode::Leaf(1);
        root.insert_leaf(2, SplitDir::Vertical, 1, 0.5);
        let rects = root.compute_rects();
        assert_eq!(rects[&1], (0.0, 0.0, 0.5, 1.0));
        assert_eq!(rects[&2], (0.5, 0.0, 0.5, 1.0));
    }

    #[test]
    fn horizontal_rects_stacked() {
        let mut root = SplitNode::Leaf(1);
        root.insert_leaf(2, SplitDir::Horizontal, 1, 0.5);
        let rects = root.compute_rects();
        assert_eq!(rects[&1], (0.0, 0.0, 1.0, 0.5));
        assert_eq!(rects[&2], (0.0, 0.5, 1.0, 0.5));
    }

    #[test]
    fn resize_leaf_adjusts_matching_divider() {
        let mut root = SplitNode::Leaf(1);
        root.insert_leaf(2, SplitDir::Vertical, 1, 0.5);
        // Divider for split 1|2 sits at x=0.5, target leaf 1.
        assert!(root.resize_leaf(1, 0.5, 0.2));
        let rects = root.compute_rects();
        assert!((rects[&1].2 - 0.7).abs() < 1e-6);
        // Wrong boundary or wrong target -> no-op.
        assert!(!root.resize_leaf(2, 0.5, 0.2));
        assert!(!root.resize_leaf(1, 0.9, 0.2));
    }

    #[test]
    fn dividers_lists_boundaries_outermost_first() {
        let mut root = SplitNode::Leaf(1);
        root.insert_leaf(2, SplitDir::Vertical, 1, 0.5);
        let divs = root.dividers();
        assert_eq!(divs.len(), 1);
        assert_eq!(divs[0], (SplitDir::Vertical, 0.5, 1));
    }

    #[test]
    fn nested_vertical_rects_share_remainder() {
        let mut root = SplitNode::Leaf(1);
        root.insert_leaf(2, SplitDir::Vertical, 1, 0.5);
        root.insert_leaf(3, SplitDir::Vertical, 1, 0.5);
        let rects = root.compute_rects();
        assert_eq!(rects[&1], (0.0, 0.0, 0.25, 1.0));
        assert_eq!(rects[&3], (0.25, 0.0, 0.25, 1.0));
        assert_eq!(rects[&2], (0.5, 0.0, 0.5, 1.0));
    }
}
