#include "auth_service.hpp"

namespace auth {

AuthService::AuthService(UserRepository& repo) : repo_(repo) {}

bool AuthService::authenticate(const std::string& email, const std::string& password) {
    User* user = repo_.findByEmail(email);
    if (user) {
        return checkPassword(user, password);
    }
    return false;
}

std::string AuthService::generateToken(const User& user) {
    return "token-" + user.email;
}

}
