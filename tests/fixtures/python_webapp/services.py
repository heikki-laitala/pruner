from models import User, Session


def authenticate_user(username, password):
    user = User.find_by_username(username)
    if user and user.check_password(password):
        session = Session.create(user.id)
        return session.token
    raise ValueError("Invalid credentials")


def get_user_profile(user_id):
    user = User.find_by_id(user_id)
    if not user:
        raise ValueError("User not found")
    return {
        "id": user.id,
        "username": user.username,
        "email": user.email,
    }
