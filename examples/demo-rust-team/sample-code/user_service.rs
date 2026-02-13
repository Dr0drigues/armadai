use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct User {
    pub id: u64,
    pub name: String,
    pub email: String,
    pub role: String,
    pub active: bool,
}

pub struct UserService {
    users: Mutex<HashMap<u64, User>>,
    next_id: Mutex<u64>,
}

impl UserService {
    pub fn new() -> Self {
        Self {
            users: Mutex::new(HashMap::new()),
            next_id: Mutex::new(1),
        }
    }

    pub fn create_user(&self, name: String, email: String, role: String) -> Result<User, String> {
        // Validate inputs
        if name.is_empty() {
            return Err("Name cannot be empty".into());
        }
        if !email.contains("@") {
            return Err("Invalid email".into());
        }

        let mut id_lock = self.next_id.lock().unwrap();
        let id = *id_lock;
        *id_lock += 1;
        drop(id_lock);

        let user = User {
            id,
            name,
            email,
            role,
            active: true,
        };

        self.users.lock().unwrap().insert(id, user.clone());
        Ok(user)
    }

    pub fn get_user(&self, id: u64) -> Option<User> {
        self.users.lock().unwrap().get(&id).cloned()
    }

    pub fn find_by_email(&self, email: &str) -> Option<User> {
        self.users
            .lock()
            .unwrap()
            .values()
            .find(|u| u.email == email)
            .cloned()
    }

    pub fn update_user(&self, id: u64, name: Option<String>, email: Option<String>) -> Result<User, String> {
        let mut users = self.users.lock().unwrap();
        let user = users.get_mut(&id).ok_or("User not found")?;

        if let Some(n) = name {
            user.name = n;
        }
        if let Some(e) = email {
            if !e.contains("@") {
                return Err("Invalid email".into());
            }
            user.email = e;
        }

        Ok(user.clone())
    }

    pub fn delete_user(&self, id: u64) -> Result<(), String> {
        let mut users = self.users.lock().unwrap();
        users.remove(&id).ok_or("User not found".into()).map(|_| ())
    }

    pub fn list_active_users(&self) -> Vec<User> {
        self.users
            .lock()
            .unwrap()
            .values()
            .filter(|u| u.active == true)
            .cloned()
            .collect()
    }

    pub fn deactivate_user(&self, id: u64) -> Result<(), String> {
        let mut users = self.users.lock().unwrap();
        match users.get_mut(&id) {
            Some(user) => {
                user.active = false;
                Ok(())
            }
            None => Err(format!("User {} not found", id)),
        }
    }

    pub fn search_users(&self, query: &str) -> Vec<User> {
        let query_lower = query.to_lowercase();
        self.users
            .lock()
            .unwrap()
            .values()
            .filter(|u| {
                u.name.to_lowercase().contains(&query_lower)
                    || u.email.to_lowercase().contains(&query_lower)
                    || u.role.to_lowercase().contains(&query_lower)
            })
            .cloned()
            .collect()
    }

    pub fn count_by_role(&self) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        for user in self.users.lock().unwrap().values() {
            *counts.entry(user.role.clone()).or_insert(0) += 1;
        }
        counts
    }

    pub fn bulk_import(&self, data: &str) -> Result<Vec<User>, String> {
        let mut imported = Vec::new();
        for line in data.lines() {
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() != 3 {
                return Err(format!("Invalid line: {}", line));
            }
            let user = self.create_user(
                parts[0].trim().to_string(),
                parts[1].trim().to_string(),
                parts[2].trim().to_string(),
            )?;
            imported.push(user);
        }
        Ok(imported)
    }
}
