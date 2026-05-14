@0xa12f5d9c3e7b4128;

# Cap'n Proto schema for the RustDesk Rendezvous protocol.
# Mirrors libs/core-common/protos/rendezvous.proto field-by-field.
#
# Design: each proto3 message becomes its own Cap'n Proto struct so that
# the union inside RendezvousMessage holds struct-pointer fields.  This is
# the idiomatic Cap'n Proto style and avoids the `group` keyword that is
# not available in all compiler back-ends.

# ── Enums ──────────────────────────────────────────────────────────────────────

enum NatType {
    unknownNat @0;
    asymmetric @1;
    symmetric  @2;
}

enum ConnType {
    defaultConn  @0;
    fileTransfer @1;
    portForward  @2;
    rdp          @3;
    viewCamera   @4;
    terminal     @5;
}

enum RegisterResult {
    ok              @0;
    uuidMismatch    @1;
    idExists        @2;
    tooFrequent     @3;
    invalidIdFormat @4;
    notSupport      @5;
    serverError     @6;
}

enum PunchHoleFailure {
    idNotExist      @0;
    offline         @1;
    licenseMismatch @2;
    licenseOveruse  @3;
}

# ── Sub-message structs ────────────────────────────────────────────────────────

struct RegisterPeer {
    id     @0 :Text;
    serial @1 :Int32;
}

struct RegisterPeerResponse {
    requestPk @0 :Bool;
}

struct PunchHoleRequest {
    id           @0 :Text;
    natType      @1 :NatType;
    licenceKey   @2 :Text;
    connType     @3 :ConnType;
    token        @4 :Text;
    version      @5 :Text;
    udpPort      @6 :Int32;
    forceRelay   @7 :Bool;
    upnpPort     @8 :Int32;
    socketAddrV6 @9 :Data;
}

struct PunchHole {
    socketAddr   @0 :Data;
    relayServer  @1 :Text;
    natType      @2 :NatType;
    udpPort      @3 :Int32;
    forceRelay   @4 :Bool;
    upnpPort     @5 :Int32;
    socketAddrV6 @6 :Data;
}

struct PunchHoleSent {
    socketAddr   @0 :Data;
    id           @1 :Text;
    relayServer  @2 :Text;
    natType      @3 :NatType;
    version      @4 :Text;
    upnpPort     @5 :Int32;
    socketAddrV6 @6 :Data;
}

struct PunchHoleResponse {
    socketAddr   @0 :Data;
    pk           @1 :Data;
    failure      @2 :PunchHoleFailure;
    relayServer  @3 :Text;
    natType      @4 :NatType;
    isLocal      @5 :Bool;
    otherFailure @6 :Text;
    feedback     @7 :Int32;
    isUdp        @8 :Bool;
    upnpPort     @9 :Int32;
    socketAddrV6 @10 :Data;
}

struct FetchLocalAddr {
    socketAddr   @0 :Data;
    relayServer  @1 :Text;
    socketAddrV6 @2 :Data;
}

struct LocalAddr {
    socketAddr   @0 :Data;
    localAddress @1 :Data;
    relayServer  @2 :Text;
    id           @3 :Text;
    version      @4 :Text;
    socketAddrV6 @5 :Data;
}

struct ConfigureUpdate {
    serial            @0 :Int32;
    rendezvousServers @1 :List(Text);
}

struct RegisterPk {
    id               @0 :Text;
    uuid             @1 :Data;
    pk               @2 :Data;
    oldId            @3 :Text;
    noRegisterDevice @4 :Bool;
    userToken        @5 :Text;
}

struct RegisterPkResponse {
    result    @0 :RegisterResult;
    keepAlive @1 :Int32;
}

struct SoftwareUpdate {
    url @0 :Text;
}

struct RequestRelay {
    id          @0 :Text;
    uuid        @1 :Text;
    socketAddr  @2 :Data;
    relayServer @3 :Text;
    secure      @4 :Bool;
    licenceKey  @5 :Text;
    connType    @6 :ConnType;
    token       @7 :Text;
}

struct RelayResponse {
    socketAddr   @0 :Data;
    uuid         @1 :Text;
    relayServer  @2 :Text;
    id           @3 :Text;
    pk           @4 :Data;
    refuseReason @5 :Text;
    version      @6 :Text;
    feedback     @7 :Int32;
    socketAddrV6 @8 :Data;
    upnpPort     @9 :Int32;
}

struct TestNatRequest {
    serial @0 :Int32;
}

# Inlines ConfigUpdate fields to avoid nesting.
struct TestNatResponse {
    port              @0 :Int32;
    configSerial      @1 :Int32;
    rendezvousServers @2 :List(Text);
}

struct PeerDiscovery {
    cmd      @0 :Text;
    mac      @1 :Text;
    id       @2 :Text;
    username @3 :Text;
    hostname @4 :Text;
    platform @5 :Text;
    misc     @6 :Text;
}

struct OnlineRequest {
    id    @0 :Text;
    peers @1 :List(Text);
}

struct OnlineResponse {
    states @0 :Data;
}

struct KeyExchange {
    keys @0 :List(Data);
}

struct HealthCheck {
    token @0 :Text;
}

struct HttpProxyRequest {
    method @0 :Text;
    path   @1 :Text;
    body   @2 :Data;
}

struct HttpProxyResponse {
    status @0 :Int32;
    body   @1 :Data;
    error  @2 :Text;
}

# ── Root union message ─────────────────────────────────────────────────────────
#
# Union discriminant ordinals @0..@22 are assigned sequentially.
# Each variant holds a pointer to its struct.

struct RendezvousMessage {
    union {
        registerPeer         @0  :RegisterPeer;
        registerPeerResponse @1  :RegisterPeerResponse;
        punchHoleRequest     @2  :PunchHoleRequest;
        punchHole            @3  :PunchHole;
        punchHoleSent        @4  :PunchHoleSent;
        punchHoleResponse    @5  :PunchHoleResponse;
        fetchLocalAddr       @6  :FetchLocalAddr;
        localAddr            @7  :LocalAddr;
        configureUpdate      @8  :ConfigureUpdate;
        registerPk           @9  :RegisterPk;
        registerPkResponse   @10 :RegisterPkResponse;
        softwareUpdate       @11 :SoftwareUpdate;
        requestRelay         @12 :RequestRelay;
        relayResponse        @13 :RelayResponse;
        testNatRequest       @14 :TestNatRequest;
        testNatResponse      @15 :TestNatResponse;
        peerDiscovery        @16 :PeerDiscovery;
        onlineRequest        @17 :OnlineRequest;
        onlineResponse       @18 :OnlineResponse;
        keyExchange          @19 :KeyExchange;
        healthCheck          @20 :HealthCheck;
        httpProxyRequest     @21 :HttpProxyRequest;
        httpProxyResponse    @22 :HttpProxyResponse;
    }
}
