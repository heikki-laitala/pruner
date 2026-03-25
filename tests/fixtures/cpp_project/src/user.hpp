#ifndef USER_HPP
#define USER_HPP

#include <string>

namespace auth {

struct User {
    int id;
    std::string email;
    std::string name;
};

class UserRepository {
public:
    User* findByEmail(const std::string& email);
    User* findById(int id);
};

}

#endif
