use crate::acceleration::SurfaceIndex;
use crate::model::{Mode, Ray};

#[derive(Clone, Copy)]
pub(super) struct Work {
    pub(super) ray: Ray,
    pub(super) next_surface: usize,
    pub(super) depth: usize,
}
pub(super) struct Traversal<'a> {
    pub(super) stack: WorkQueue,
    pub(super) surface_index: Option<&'a SurfaceIndex>,
}
pub(super) struct WeightedWork {
    work: Work,
    order: u64,
}
impl PartialEq for WeightedWork {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl Eq for WeightedWork {}
impl PartialOrd for WeightedWork {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for WeightedWork {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.work
            .ray
            .weight
            .total_cmp(&other.work.ray.weight)
            .then(self.order.cmp(&other.order))
    }
}
pub(super) enum WorkQueue {
    Sequential(Option<Work>),
    NonSequential(std::collections::BinaryHeap<WeightedWork>, u64),
}
impl WorkQueue {
    pub(super) fn new(mode: Mode) -> Self {
        if mode == Mode::Sequential {
            Self::Sequential(None)
        } else {
            Self::NonSequential(std::collections::BinaryHeap::new(), 0)
        }
    }
    pub(super) fn start(&mut self, first: Work) {
        if let Self::NonSequential(q, order) = self {
            debug_assert!(q.is_empty());
            *order = 0;
        }
        self.push(first);
    }
    pub(super) fn push(&mut self, work: Work) {
        match self {
            Self::Sequential(q) => {
                debug_assert!(q.is_none());
                *q = Some(work);
            }
            Self::NonSequential(q, order) => {
                q.push(WeightedWork {
                    work,
                    order: *order,
                });
                *order += 1;
            }
        }
    }
    pub(super) fn pop(&mut self) -> Option<Work> {
        match self {
            Self::Sequential(q) => q.take(),
            Self::NonSequential(q, _) => q.pop().map(|w| w.work),
        }
    }
}
