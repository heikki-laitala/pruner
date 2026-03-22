from services import authenticate_user, get_user_profile


def test_authenticate_user():
    token = authenticate_user("admin", "secret")
    assert token is not None


def test_authenticate_user_invalid():
    try:
        authenticate_user("admin", "wrong")
    except ValueError:
        pass


def test_get_user_profile():
    profile = get_user_profile(1)
    assert profile["username"] is not None
