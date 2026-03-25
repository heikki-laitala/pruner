#include <string.h>
#include <stdlib.h>
#include "auth.h"

int authenticate(const char* email, const char* password, UserStore* store) {
    User* user = find_by_email(store, email);
    if (user && check_password(user, password)) {
        return 1;
    }
    return 0;
}

char* generate_token(const User* user) {
    char* token = malloc(256);
    snprintf(token, 256, "token-%s", user->email);
    return token;
}
