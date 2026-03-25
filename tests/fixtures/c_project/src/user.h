#ifndef USER_H
#define USER_H

typedef struct {
    int id;
    char* email;
    char* name;
} User;

typedef struct {
    User* users;
    int count;
} UserStore;

UserStore* create_user_store(void);
void free_user_store(UserStore* store);
User* find_by_email(UserStore* store, const char* email);
User* find_by_id(UserStore* store, int id);
int check_password(const User* user, const char* password);

#endif
