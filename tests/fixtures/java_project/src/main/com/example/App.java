package com.example;

import com.example.handlers.AuthHandler;
import com.example.models.UserRepository;

public class App {
    public static void main(String[] args) {
        UserRepository repo = new UserRepository();
        AuthHandler handler = new AuthHandler(repo);
        handler.start();
    }
}
