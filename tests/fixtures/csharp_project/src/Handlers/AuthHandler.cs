using System;
using CSharpProject.Models;
using CSharpProject.Repositories;

namespace CSharpProject.Handlers
{
    public class AuthHandler
    {
        private readonly UserRepository _repository;

        public AuthHandler(UserRepository repository)
        {
            _repository = repository;
        }

        public User Authenticate(int userId, string password)
        {
            var user = _repository.FindById(userId);
            if (user == null)
            {
                return null;
            }
            return ValidatePassword(user, password);
        }

        private User ValidatePassword(User user, string password)
        {
            return user;
        }
    }
}
