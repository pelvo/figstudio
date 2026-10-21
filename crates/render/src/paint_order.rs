//! Shared stacking order for drawing and inspecting immediate children.
use figstudio_fig::{Guid, Val};

/// Topmost first, opposite to painting. Reverse this iterator to paint bottom to top.
/// Filtering and budget truncation belong to the caller, after choosing the order.
pub fn children_topmost_first<'a>(
    children: &'a [Guid],
    data: &Val,
) -> impl DoubleEndedIterator<Item = Guid> + 'a {
    let reverse = data.flag("stackReverseZIndex") == Some(true);
    (0..children.len()).map(move |i| children[if reverse { i } else { children.len() - 1 - i }])
}
