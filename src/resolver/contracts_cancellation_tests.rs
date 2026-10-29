use super::*;

#[test]
fn child_cancel_is_local_and_clones_share_only_their_own_scope() {
    let parent = ResolverCancellation::default();
    let child = ResolverCancellation::linked(parent.clone()).unwrap();
    let sibling = ResolverCancellation::linked(parent.clone()).unwrap();
    let clone = child.clone();
    assert!(clone.same_scope(&child));
    assert!(!parent.same_scope(&child));
    assert!(!sibling.same_scope(&child));
    child.cancel();
    assert!(clone.is_cancelled());
    assert!(!parent.is_cancelled());
    assert!(!sibling.is_cancelled());
}

#[tokio::test]
async fn already_retired_parent_is_observed_without_a_relay_task() {
    let parent = ResolverCancellation::default();
    parent.cancel();
    let child = ResolverCancellation::linked(parent).unwrap();
    assert!(child.is_cancelled());
    tokio::time::timeout(Duration::from_secs(5), child.cancelled())
        .await
        .unwrap();
}

#[tokio::test]
async fn waiting_child_wakes_for_every_ancestor_and_its_local_scope() {
    for cancelled_scope in 0..=MAX_CANCELLATION_PARENTS {
        let mut scopes = vec![ResolverCancellation::default()];
        for _ in 0..MAX_CANCELLATION_PARENTS {
            scopes.push(ResolverCancellation::linked(scopes.last().unwrap().clone()).unwrap());
        }
        let leaf = scopes.last().unwrap().clone();
        let waiting = leaf.cancelled();
        tokio::pin!(waiting);
        tokio::select! { biased;
            _ = &mut waiting => panic!("uncancelled scope completed"),
            _ = tokio::task::yield_now() => {},
        }
        scopes[cancelled_scope].cancel();
        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .unwrap();
        assert!(leaf.is_cancelled());
        for ancestor in scopes.iter().take(cancelled_scope) {
            assert!(!ancestor.is_cancelled());
        }
    }
}

#[test]
fn ancestry_is_flat_and_fifth_distinct_parent_is_rejected() {
    let mut child = ResolverCancellation::default();
    for count in 1..=MAX_CANCELLATION_PARENTS {
        child = ResolverCancellation::linked(child).unwrap();
        assert_eq!(child.0.parents.len(), count);
    }
    assert!(matches!(
        ResolverCancellation::linked(child),
        Err(ResolverError::InvalidData)
    ));
}

#[test]
fn combined_ancestry_deduplicates_shared_channel_snapshots() {
    let root = ResolverCancellation::default();
    let first = ResolverCancellation::linked(root.clone()).unwrap();
    let second = ResolverCancellation::linked(root.clone()).unwrap();
    let combined = ResolverCancellation::linked_many(&[
        first.clone(),
        second.clone(),
        root.clone(),
        first.clone(),
    ])
    .unwrap();
    assert_eq!(combined.0.parents.len(), 3);
    assert!(!combined.same_scope(&root));
    assert!(!combined.same_scope(&first));
    second.cancel();
    assert!(combined.is_cancelled());
    assert!(!first.is_cancelled());
    assert!(!root.is_cancelled());
}

#[tokio::test]
async fn dropping_parent_handle_does_not_retire_retained_ancestor_channel() {
    let child = {
        let parent = ResolverCancellation::default();
        ResolverCancellation::linked(parent).unwrap()
    };
    assert!(!child.is_cancelled());
    let waiting = child.cancelled();
    tokio::pin!(waiting);
    tokio::select! { biased;
        _ = &mut waiting => panic!("retained parent channel was closed"),
        _ = tokio::task::yield_now() => {},
    }
    child.cancel();
    tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .unwrap();
}
