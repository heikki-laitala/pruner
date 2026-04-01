namespace CSharpProject.Models;

public class User
{
    public int Id { get; set; }
    public string Username { get; set; } = string.Empty;
    public string Email { get; set; } = string.Empty;

    public User(int id, string username, string email)
    {
        Id = id;
        Username = username;
        Email = email;
    }

    public override string ToString()
    {
        return $"User({Id}, {Username})";
    }
}
