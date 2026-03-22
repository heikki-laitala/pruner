from services import authenticate_user, get_user_profile


def login_handler(request):
    token = authenticate_user(request.username, request.password)
    return {"token": token}


def profile_handler(request):
    profile = get_user_profile(request.user_id)
    return {"profile": profile}


def health_check():
    return {"status": "ok"}
