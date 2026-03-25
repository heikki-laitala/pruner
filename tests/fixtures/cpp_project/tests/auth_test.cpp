#include <cassert>
#include "../src/auth_service.hpp"

void test_authenticate() {
    auth::UserRepository repo;
    auth::AuthService service(repo);
    assert(service.authenticate("admin@example.com", "secret"));
}

int main() {
    test_authenticate();
    return 0;
}
