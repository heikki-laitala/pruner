package com.example.handlers;

import com.example.models.User;
import com.example.models.UserRepository;

public class AuthHandler {
    private final UserRepository userRepo;

    public AuthHandler(UserRepository userRepo) {
        this.userRepo = userRepo;
    }

    public User authenticate(String email, String password) {
        User user = userRepo.findByEmail(email);
        if (user != null && user.checkPassword(password)) {
            return user;
        }
        return null;
    }

    public String generateToken(User user) {
        return "token-" + user.getEmail();
    }

    public void start() {
        System.out.println("AuthHandler started");
    }
}
