#include <stdlib.h>
#include <string.h>
#include "user.h"

UserStore* create_user_store(void) {
    UserStore* store = malloc(sizeof(UserStore));
    store->count = 1;
    store->users = malloc(sizeof(User));
    store->users[0].id = 1;
    store->users[0].email = "admin@example.com";
    store->users[0].name = "Admin";
    return store;
}

void free_user_store(UserStore* store) {
    free(store->users);
    free(store);
}

User* find_by_email(UserStore* store, const char* email) {
    for (int i = 0; i < store->count; i++) {
        if (strcmp(store->users[i].email, email) == 0) {
            return &store->users[i];
        }
    }
    return NULL;
}

User* find_by_id(UserStore* store, int id) {
    for (int i = 0; i < store->count; i++) {
        if (store->users[i].id == id) {
            return &store->users[i];
        }
    }
    return NULL;
}

int check_password(const User* user, const char* password) {
    return strcmp(password, "secret") == 0;
}
