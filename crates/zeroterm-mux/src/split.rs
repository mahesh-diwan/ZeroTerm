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

    /// Persistence form: map every leaf pane id onto its position in `ids`
    /// (the saved pane list, sorted by id). `remap_split` on restore rebases
    /// those positions onto freshly assigned ids, so a tree survives a
    /// session save/restore round trip. Unknown ids map to position 0 (a
    /// leaf that vanished between save and restore stays well-formed).
    pub fn to_positions(&self, ids: &[usize]) -> Self {
        match self {
            SplitNode::Leaf(id) => SplitNode::Leaf(ids.iter().position(|i| i == id).unwrap_or(0)),
            SplitNode::Split {
                dir,
                children,
                ratio,
            } => SplitNode::Split {
                dir: *dir,
                children: children.iter().map(|c| c.to_positions(ids)).collect(),
                ratio: *ratio,
            },
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
                    let dir = if depth.is_multiple_of(2) {
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

    /// Pane id whose normalized rect contains the content-space point (x, y),
    /// both in 0..=1 (the same space compute_rects emits). Rects tile the unit
    /// square without overlap, so a point inside the content area hits exactly
    /// one pane; None outside every rect. Used for mouse hit-testing.
    pub fn pane_at(&self, x: f32, y: f32) -> Option<usize> {
        self.compute_rects()
            .into_iter()
            .find_map(|(id, (px, py, pw, ph))| {
                (x >= px && y >= py && x < px + pw && y < py + ph).then_some(id)
            })
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

    /// Divider lines as (dir, normalized boundary position, first-leaf of the
    /// second child). Boundary position is in the normalized 0..1 content space
    /// (x for Vertical, y for Horizontal), matching compute_rects. DFS order,
    /// outermost first.
    ///
    /// The target leaf uniquely identifies a divider: each divider separates
    /// children[0] from children[1..], so its target is children[1]'s first
    /// leaf, which no other divider in the tree can produce (the second-child
    /// subtrees of nested splits are disjoint).
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
                        if let Some(target) =
                            children.get(1).and_then(|c| c.leaves().first().copied())
                        {
                            out.push((SplitDir::Vertical, x + first_w, target));
                        }
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
                        if let Some(target) =
                            children.get(1).and_then(|c| c.leaves().first().copied())
                        {
                            out.push((SplitDir::Horizontal, y + first_h, target));
                        }
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
    /// second-child first-leaf is `target` (the target emitted by dividers()).
    /// `delta` is the normalized move amount (positive grows the first child).
    /// Clamps ratio to 0.15..0.85.
    pub fn resize_leaf(&mut self, target: usize, boundary: f32, delta: f32) -> bool {
        self.resize_walk(0.0, 0.0, 1.0, 1.0, target, boundary, delta)
    }

    // 7 recursive-geometry args; a context struct adds churn for no clarity.
    #[allow(clippy::too_many_arguments)]
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
                if (divider_pos - boundary).abs() < 0.001
                    && children.get(1).and_then(|c| c.leaves().first().copied()) == Some(target)
                {
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
    fn to_positions_round_trips_through_remap() {
        let root = SplitNode::from_ids(&[3, 1, 4, 2]);
        let ids = vec![1, 2, 3, 4];
        let positions = root.to_positions(&ids);
        assert_eq!(positions.leaves(), vec![2, 0, 3, 1]);
        // Unknown leaf maps to position 0, keeping the tree well-formed.
        let bad = SplitNode::Leaf(99);
        assert_eq!(bad.to_positions(&[1, 2]).leaves(), vec![0]);
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
        // Divider for split 1|2 sits at x=0.5, target leaf 2 (first leaf of the
        // second child).
        assert!(root.resize_leaf(2, 0.5, 0.2));
        let rects = root.compute_rects();
        assert!((rects[&1].2 - 0.7).abs() < 1e-6);
        // Wrong boundary or wrong target -> no-op.
        assert!(!root.resize_leaf(1, 0.5, 0.2));
        assert!(!root.resize_leaf(2, 0.9, 0.2));
    }

    #[test]
    fn dividers_lists_boundaries_outermost_first() {
        let mut root = SplitNode::Leaf(1);
        root.insert_leaf(2, SplitDir::Vertical, 1, 0.5);
        let divs = root.dividers();
        assert_eq!(divs.len(), 1);
        assert_eq!(divs[0], (SplitDir::Vertical, 0.5, 2));
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

    #[test]
    fn pane_at_hits_all_quadrants_of_2x2() {
        // from_ids([1,2,3,4]): left column [1,2] split H, right column [3,4].
        let root = SplitNode::from_ids(&[1, 2, 3, 4]);
        assert_eq!(root.pane_at(0.25, 0.25), Some(1));
        assert_eq!(root.pane_at(0.25, 0.75), Some(2));
        assert_eq!(root.pane_at(0.75, 0.25), Some(3));
        assert_eq!(root.pane_at(0.75, 0.75), Some(4));
        // Divider boundaries belong to the second child (left/half-open rects).
        assert_eq!(root.pane_at(0.5, 0.25), Some(3));
        assert_eq!(root.pane_at(0.25, 0.5), Some(2));
        // Outside the unit square -> None.
        assert_eq!(root.pane_at(1.5, 0.5), None);
        assert_eq!(root.pane_at(-0.1, 0.5), None);
    }

    // --- divider target uniqueness / drag resolution ---

    /// main.rs resolves a divider drag with `.find(|(_, _, t)| *t == target)`.
    /// If two dividers share a target, the outer one always wins and dragging an
    /// inner divider silently resizes the wrong split.
    #[test]
    fn divider_targets_are_unique() {
        let root = SplitNode::from_ids(&[1, 2, 3, 4]);
        let mut targets: Vec<usize> = root.dividers().into_iter().map(|(_, _, t)| t).collect();
        targets.sort_unstable();
        assert!(
            targets.windows(2).all(|w| w[0] != w[1]),
            "divider targets must uniquely identify each divider: {:?}",
            targets
        );
    }

    /// Dragging the left column's inner horizontal divider must resize the left
    /// column's ratio only — not the outer vertical split.
    #[test]
    fn nested_divider_drag_resizes_correct_split() {
        let mut root = SplitNode::from_ids(&[1, 2, 3, 4]);
        // Resolve the inner H divider (pane1 | pane2) the way main.rs does.
        let (dir, boundary) = root
            .dividers()
            .into_iter()
            .find(|(_, _, t)| *t == 2)
            .map(|(d, b, _)| (d, b))
            .expect("inner divider must be resolvable by its target");
        assert_eq!(dir, SplitDir::Horizontal);
        assert!((boundary - 0.5).abs() < 1e-6);
        assert!(root.resize_leaf(2, boundary, 0.1));
        let rects = root.compute_rects();
        assert!(
            (rects[&1].3 - 0.6).abs() < 1e-6,
            "pane1 height must grow to 0.6, got {}",
            rects[&1].3
        );
        assert!(
            (rects[&2].1 - 0.6).abs() < 1e-6,
            "pane2 must start at y=0.6, got {}",
            rects[&2].1
        );
        assert!(
            (rects[&1].2 - 0.5).abs() < 1e-6,
            "outer vertical split must stay 0.5 wide, got {}",
            rects[&1].2
        );
        assert!(
            (rects[&3].3 - 0.5).abs() < 1e-6,
            "right column must be untouched, got {}",
            rects[&3].3
        );
    }

    /// Every divider's boundary must equal the rect start of the pane it points
    /// at (the first leaf of the second child), tying dividers() to compute_rects.
    #[test]
    fn divider_boundaries_align_with_compute_rects() {
        let root = SplitNode::from_ids(&[1, 2, 3, 4]);
        let rects = root.compute_rects();
        for (dir, boundary, target) in root.dividers() {
            match dir {
                SplitDir::Vertical => assert!(
                    (rects[&target].0 - boundary).abs() < 1e-5,
                    "V divider {} must match pane {} x start {}",
                    boundary,
                    target,
                    rects[&target].0
                ),
                SplitDir::Horizontal => assert!(
                    (rects[&target].1 - boundary).abs() < 1e-5,
                    "H divider {} must match pane {} y start {}",
                    boundary,
                    target,
                    rects[&target].1
                ),
            }
        }
    }

    // --- removal consistency ---

    #[test]
    fn remove_leaf_various_orders_keep_tree_consistent() {
        let orders: [&[usize]; 4] = [&[2, 3], &[3, 2], &[2, 4], &[4, 2]];
        for order in orders {
            let mut root = SplitNode::from_ids(&[1, 2, 3, 4]);
            for &id in order {
                root.remove_leaf(id);
                let mut leaves = root.leaves();
                leaves.sort_unstable();
                let mut keys: Vec<usize> = root.compute_rects().keys().copied().collect();
                keys.sort_unstable();
                assert_eq!(
                    keys, leaves,
                    "rects keys must match leaves after removing {}",
                    id
                );
                let rects = root.compute_rects();
                let sum_area: f32 = rects.values().map(|r| r.2 * r.3).sum();
                assert!(
                    (sum_area - 1.0).abs() < 1e-4,
                    "leaf rects must tile the unit square after removing {}, got area {}",
                    id,
                    sum_area
                );
                for (pid, &(x, y, w, h)) in &rects {
                    assert!(
                        x >= -1e-5 && y >= -1e-5 && x + w <= 1.0 + 1e-5 && y + h <= 1.0 + 1e-5,
                        "pane {} rect {:?} must stay inside the unit square",
                        pid,
                        (x, y, w, h)
                    );
                    assert!(
                        w > 0.0 && h > 0.0,
                        "pane {} must keep a positive rect, got {}x{}",
                        pid,
                        w,
                        h
                    );
                }
            }
        }
    }

    #[test]
    fn remove_leaf_last_pane_is_noop() {
        let mut root = SplitNode::Leaf(5);
        root.remove_leaf(5);
        assert_eq!(root.leaves(), vec![5]);
        root.remove_leaf(7);
        assert_eq!(root.leaves(), vec![5]);
    }

    // --- resize clamping ---

    #[test]
    fn resize_ratio_clamps_without_accumulating() {
        let mut root = SplitNode::from_ids(&[1, 2]);
        assert!(root.resize_leaf(2, 0.5, -0.9));
        let rects = root.compute_rects();
        assert!((rects[&1].2 - 0.15).abs() < 1e-6);
        assert!((rects[&2].2 - 0.85).abs() < 1e-6);
        // Further drags past the clamp must not shrink below the bound.
        assert!(root.resize_leaf(2, 0.15, -0.5));
        let rects = root.compute_rects();
        assert!((rects[&1].2 - 0.15).abs() < 1e-6);
    }

    // --- insert semantics ---

    #[test]
    fn insert_into_unknown_parent_is_noop() {
        let mut root = SplitNode::from_ids(&[1, 2]);
        root.insert_leaf(9, SplitDir::Vertical, 999, 0.5);
        assert_eq!(root.leaves(), vec![1, 2]);
    }

    #[test]
    fn inserted_pane_becomes_last_child_of_parent() {
        let mut root = SplitNode::from_ids(&[1, 2, 3]);
        root.insert_leaf(4, SplitDir::Vertical, 2, 0.5);
        assert_eq!(root.leaves(), vec![1, 2, 4, 3]);
    }
}
