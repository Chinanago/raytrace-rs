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
    pub(super) fn start(&mut self, first: Work) -> crate::Result<()> {
        if let Self::NonSequential(q, order) = self {
            if !q.is_empty() {
                return Err(crate::Error::Internal(
                    "追跡キューに未処理の光線が残っています".into(),
                ));
            }
            *order = 0;
        }
        self.push(first)
    }
    pub(super) fn push(&mut self, work: Work) -> crate::Result<()> {
        match self {
            Self::Sequential(q) => {
                if q.is_some() {
                    return Err(crate::Error::Internal("逐次追跡キューが使用中です".into()));
                }
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
        Ok(())
    }
    pub(super) fn pop(&mut self) -> Option<Work> {
        match self {
            Self::Sequential(q) => q.take(),
            Self::NonSequential(q, _) => q.pop().map(|w| w.work),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::V3;

    fn work(id: u64) -> Work {
        Work {
            ray: Ray {
                id,
                position_mm: V3([0.; 3]),
                direction: V3([0., 0., 1.]),
                weight: 1.,
                wavelength_um: 0.55,
                medium: 0,
                optical_path_mm: 0.,
            },
            next_surface: 0,
            depth: 0,
        }
    }

    #[test]
    fn busy_queues_return_errors_without_losing_pending_work() {
        let mut sequential = WorkQueue::new(Mode::Sequential);
        sequential.start(work(1)).unwrap();
        assert!(matches!(
            sequential.push(work(2)),
            Err(crate::Error::Internal(_))
        ));
        assert_eq!(sequential.pop().unwrap().ray.id, 1);
        assert!(sequential.pop().is_none());

        let mut non_sequential = WorkQueue::new(Mode::NonSequential);
        non_sequential.start(work(3)).unwrap();
        assert!(matches!(
            non_sequential.start(work(4)),
            Err(crate::Error::Internal(_))
        ));
        assert_eq!(non_sequential.pop().unwrap().ray.id, 3);
        non_sequential.start(work(5)).unwrap();
        assert_eq!(non_sequential.pop().unwrap().ray.id, 5);
    }
}
