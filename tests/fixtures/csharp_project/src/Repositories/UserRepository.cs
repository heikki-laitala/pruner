using System.Collections.Generic;
using CSharpProject.Models;

namespace CSharpProject.Repositories
{
    public class UserRepository
    {
        private readonly List<User> _users = new List<User>();

        public User FindById(int id)
        {
            return _users.Find(u => u.Id == id);
        }

        public void Save(User user)
        {
            _users.Add(user);
        }
    }
}
