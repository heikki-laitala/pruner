#include <stdio.h>
#include "auth.h"
#include "user.h"

int main(int argc, char* argv[]) {
    UserStore* store = create_user_store();
    int result = authenticate("admin@example.com", "secret", store);
    printf("Auth result: %d\n", result);
    free_user_store(store);
    return 0;
}
