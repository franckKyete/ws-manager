pub struct RollbackAction {
    pub description: String,
    pub action: Box<dyn FnOnce() + Send>,
}

pub struct RollbackStack {
    actions: Vec<RollbackAction>,
}

impl Default for RollbackStack {
    fn default() -> Self {
        Self::new()
    }
}

impl RollbackStack {
    pub fn new() -> Self {
        Self {
            actions: Vec::new(),
        }
    }

    pub fn add<F>(&mut self, description: impl Into<String>, action: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.actions.push(RollbackAction {
            description: description.into(),
            action: Box::new(action),
        });
    }

    pub fn clear(&mut self) {
        self.actions.clear();
    }

    pub fn execute(&mut self) -> Vec<String> {
        let mut executed = Vec::new();
        while let Some(action) = self.actions.pop() {
            let desc = action.description;
            (action.action)();
            executed.push(desc);
        }
        executed
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_rollback_lifo_order() {
        let mut stack = RollbackStack::new();
        let log = Arc::new(Mutex::new(Vec::new()));

        let log_clone1 = Arc::clone(&log);
        stack.add("action 1", move || {
            log_clone1.lock().unwrap().push(1);
        });

        let log_clone2 = Arc::clone(&log);
        stack.add("action 2", move || {
            log_clone2.lock().unwrap().push(2);
        });

        let executed = stack.execute();
        assert_eq!(executed, vec!["action 2", "action 1"]);
        assert_eq!(*log.lock().unwrap(), vec![2, 1]);
        assert!(stack.is_empty());
    }

    #[test]
    fn test_rollback_clear() {
        let mut stack = RollbackStack::new();
        stack.add("action 1", || {});
        assert!(!stack.is_empty());
        stack.clear();
        assert!(stack.is_empty());
        let executed = stack.execute();
        assert!(executed.is_empty());
    }
}
