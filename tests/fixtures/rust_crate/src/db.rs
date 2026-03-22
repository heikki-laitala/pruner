pub struct Pool {
    pub url: String,
}

pub struct User {
    pub id: i64,
    pub username: String,
}

pub struct Session {
    pub token: String,
}

pub fn create_pool(url: &str) -> Pool {
    Pool { url: url.to_string() }
}

pub fn get_user(pool: &Pool, username: &str) -> Option<User> {
    let _ = (pool, username);
    None
}

pub fn create_session(pool: &Pool, user_id: i64) -> Session {
    let _ = (pool, user_id);
    Session { token: "abc123".to_string() }
}
