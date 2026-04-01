using System;
using CSharpProject.Handlers;
using CSharpProject.Models;
using CSharpProject.Repositories;

namespace CSharpProject.Tests
{
    public class AuthHandlerTests
    {
        public void TestAuthenticate()
        {
            var repo = new UserRepository();
            var user = new User(1, "alice", "alice@example.com");
            repo.Save(user);
            var handler = new AuthHandler(repo);
            var result = handler.Authenticate(1, "correct");
            Console.WriteLine(result);
        }
    }
}
