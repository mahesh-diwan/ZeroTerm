//! Split - tiling tree of panes within a tab

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDir {
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone)]
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
