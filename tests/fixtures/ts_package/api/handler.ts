import { validateToken, createSession } from '../lib/auth';
import { findUser, updateLastLogin } from '../lib/db';

export async function handleLogin(req: Request): Promise<Response> {
    const { username, password } = await req.json();
    const user = await findUser(username);
    if (!user) {
        return new Response('Not found', { status: 404 });
    }
    const session = await createSession(user.id);
    await updateLastLogin(user.id);
    return new Response(JSON.stringify({ token: session.token }));
}

export async function handleProfile(req: Request): Promise<Response> {
    const token = req.headers.get('Authorization');
    const userId = await validateToken(token);
    const user = await findUser(userId);
    return new Response(JSON.stringify(user));
}

export function handleHealthCheck(): Response {
    return new Response('ok');
}
