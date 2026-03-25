#ifndef AUTH_H
#define AUTH_H

#include "user.h"

int authenticate(const char* email, const char* password, UserStore* store);
char* generate_token(const User* user);

#endif
