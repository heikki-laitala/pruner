#include <iostream>
#include "auth_service.hpp"
#include "user.hpp"

int main() {
    auth::UserRepository repo;
    auth::AuthService service(repo);
    auto result = service.authenticate("admin@example.com", "secret");
    std::cout << "Auth: " << result << std::endl;
    return 0;
}
