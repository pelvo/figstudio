use std::sync::Arc;
use std::time::Duration;

use super::*;

#[test]
fn workspace_order_is_independent_of_decode_order_and_respects_a_new_focus() {
    let h = crate::ui_tests::harness();
    let doc = Arc::clone(&h.state().tabs[0].doc);
    let paths: Vec<_> = ["first.fig", "second.fig", "third.fig"]
        .map(PathBuf::from)
        .into();
    for manually_selected in [false, true] {
        let mut restore = Restore::new(Session {
            files: paths.clone(),
            active: Some(paths[1].clone()),
        });
        let mut tabs = Vec::new();
        let mut active = 0;
        for i in [2, 0, 1] {
            let previous = tabs.get(active).map(|t: &Tab| t.id);
            tabs.push(Tab::new(
                i as u64,
                paths[i].clone(),
                Arc::clone(&doc),
                None,
                Duration::ZERO,
            ));
            active = tabs.len() - 1;
            restore.finished(&mut tabs, &mut active, &paths[i], previous, None);
            if i == 0 && manually_selected {
                active = 0;
            }
        }
        assert_eq!(
            tabs.iter().map(|t| &t.path).collect::<Vec<_>>(),
            paths.iter().collect::<Vec<_>>()
        );
        assert_eq!(tabs[active].path, paths[usize::from(!manually_selected)]);
    }
}
