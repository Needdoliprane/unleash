// Ce fichier a été généré automatiquement par unleash-protocol (ts-rs).
// Ne pas modifier manuellement.

export type HelloRequest = { 
/**
 * Version protocolaire annoncée par le client (`unleash.ipc.v2`).
 */
protocol_version: ProtocolVersion, 
/**
 * Nature du client.
 */
client_kind: ClientKind, 
/**
 * Version du binaire client.
 */
client_version: string, 
/**
 * Capacités optionnelles requises ou supportées par le client.
 */
capabilities: Array<string>, };
