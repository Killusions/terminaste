use super::*;

#[derive(Clone, Serialize, Deserialize)]
pub(super) enum PaneTree {
    Leaf(Uuid),
    Split {
        id: Uuid,
        direction: SplitDirection,
        ratio: f32,
        first: Box<PaneTree>,
        second: Box<PaneTree>,
    },
}

impl PaneTree {
    pub fn split(&mut self, target: Uuid, new: Uuid, direction: SplitDirection) {
        match self {
            Self::Leaf(id) if *id == target => {
                *self = Self::Split {
                    id: Uuid::new_v4(),
                    direction,
                    ratio: 0.5,
                    first: Box::new(Self::Leaf(target)),
                    second: Box::new(Self::Leaf(new)),
                };
            }
            Self::Split { first, second, .. } => {
                first.split(target, new, direction);
                second.split(target, new, direction);
            }
            Self::Leaf(_) => {}
        }
    }

    pub fn resize(&mut self, target: Uuid, value: f32) {
        if let Self::Split {
            id,
            ratio,
            first,
            second,
            ..
        } = self
        {
            if *id == target {
                *ratio = value.clamp(0.1, 0.9);
            } else {
                first.resize(target, value);
                second.resize(target, value);
            }
        }
    }
}
