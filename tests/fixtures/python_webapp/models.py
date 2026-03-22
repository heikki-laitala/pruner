class User:
    def __init__(self, id, username, email, password_hash):
        self.id = id
        self.username = username
        self.email = email
        self.password_hash = password_hash

    @staticmethod
    def find_by_username(username):
        pass

    @staticmethod
    def find_by_id(user_id):
        pass

    def check_password(self, password):
        pass


class Session:
    def __init__(self, user_id, token):
        self.user_id = user_id
        self.token = token

    @staticmethod
    def create(user_id):
        pass
