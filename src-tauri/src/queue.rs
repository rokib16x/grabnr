//! Queue order: which waiting download starts next, and how a drag in the list changes it.

/// One download as the queue sees it. `age` is its position in the list, newest = 0, so a larger age is older.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub priority: i32,
    pub age: usize,
}

/// Highest priority first, then oldest first. The same rule the downloader uses to pick what starts next.
pub fn order(entries: &[Entry]) -> Vec<String> {
    let mut v: Vec<&Entry> = entries.iter().collect();
    v.sort_by(|a, b| b.priority.cmp(&a.priority).then(b.age.cmp(&a.age)));
    v.into_iter().map(|e| e.id.clone()).collect()
}

/// Move `id` in front of `before` (or to the end when `before` is `None`).
pub fn move_before(mut order: Vec<String>, id: &str, before: Option<&str>) -> Vec<String> {
    if before == Some(id) {
        return order;
    }
    let Some(from) = order.iter().position(|x| x == id) else { return order };
    let item = order.remove(from);
    let at = before.and_then(|b| order.iter().position(|x| x == b)).unwrap_or(order.len());
    order.insert(at, item);
    order
}

/// Priorities that reproduce `order` exactly: the first gets the highest number.
pub fn priorities(order: &[String]) -> Vec<(String, i32)> {
    let n = order.len() as i32;
    order.iter().enumerate().map(|(i, id)| (id.clone(), n - i as i32)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(id: &str, priority: i32, age: usize) -> Entry {
        Entry { id: id.into(), priority, age }
    }

    #[test]
    fn oldest_first_then_priority() {
        let v = vec![e("new", 0, 0), e("mid", 0, 1), e("old", 0, 2)];
        assert_eq!(order(&v), ["old", "mid", "new"]);
        let v = vec![e("new", 5, 0), e("mid", 0, 1), e("old", 0, 2)];
        assert_eq!(order(&v), ["new", "old", "mid"]);
    }

    #[test]
    fn dragging_reorders() {
        let o: Vec<String> = ["a", "b", "c", "d"].map(String::from).to_vec();
        assert_eq!(move_before(o.clone(), "d", Some("b")), ["a", "d", "b", "c"]);
        assert_eq!(move_before(o.clone(), "a", None), ["b", "c", "d", "a"]);
        assert_eq!(move_before(o.clone(), "b", Some("b")), ["a", "b", "c", "d"], "dropping on itself changes nothing");
        assert_eq!(move_before(o.clone(), "zzz", Some("a")), o, "unknown ids are ignored");
    }

    #[test]
    fn priorities_reproduce_the_order() {
        let o: Vec<String> = ["x", "y", "z"].map(String::from).to_vec();
        let p = priorities(&o);
        let entries: Vec<Entry> = p.iter().enumerate().map(|(i, (id, pr))| e(id, *pr, i)).collect();
        assert_eq!(order(&entries), o);
    }
}
