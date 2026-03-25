#ifndef AUTH_SERVICE_HPP
#define AUTH_SERVICE_HPP

#include <string>
#include "user.hpp"

namespace auth {

class AuthService {
public:
    AuthService(UserRepository& repo);
    bool authenticate(const std::string& email, const std::string& password);
    std::string generateToken(const User& user);

private:
    UserRepository& repo_;
};

}

#endif
