import { findSession, createSessionRecord } from './db';

export interface Session {
    id: string;
    userId: string;
    token: string;
    expiresAt: Date;
}

export async function validateToken(token: string | null): Promise<string> {
    if (!token) {
        throw new Error('No token provided');
    }
    const session = await findSession(token);
    if (!session || session.expiresAt < new Date()) {
        throw new Error('Invalid or expired token');
    }
    return session.userId;
}

export async function createSession(userId: string): Promise<Session> {
    const token = generateToken();
    const session = await createSessionRecord(userId, token);
    return session;
}

function generateToken(): string {
    return Math.random().toString(36).substring(2);
}
