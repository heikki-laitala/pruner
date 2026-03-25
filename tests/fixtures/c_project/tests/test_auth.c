#include <assert.h>
#include "../src/auth.h"
#include "../src/user.h"

void test_authenticate(void) {
    UserStore* store = create_user_store();
    assert(authenticate("admin@example.com", "secret", store) == 1);
    assert(authenticate("unknown@example.com", "secret", store) == 0);
    free_user_store(store);
}

int main(void) {
    test_authenticate();
    return 0;
}
