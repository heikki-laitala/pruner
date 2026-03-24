package com.example.handlers;

import com.example.models.User;
import com.example.models.UserRepository;

public class AuthHandlerTest {
    public void testAuthenticate() {
        UserRepository repo = new UserRepository();
        AuthHandler handler = new AuthHandler(repo);
        User user = handler.authenticate("admin@example.com", "password");
        assert user != null;
    }

    public void testGenerateToken() {
        User user = new User(1, "test@example.com", "Test");
        AuthHandler handler = new AuthHandler(new UserRepository());
        String token = handler.generateToken(user);
        assert token != null;
    }
}
