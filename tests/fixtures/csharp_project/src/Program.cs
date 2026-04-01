using CSharpProject.Handlers;
using CSharpProject.Repositories;

var repo = new UserRepository();
var handler = new AuthHandler(repo);
handler.Authenticate(1, "secret");
