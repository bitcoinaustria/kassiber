use crate::{Error, Operation, Transport};
use std::io::{Read, Write};
use std::net::IpAddr;
use std::time::Instant;

fn send(connection: &mut Transport, bytes: &[u8], deadline: Instant) -> Result<(), Error> {
    connection.deadline = Some(deadline);
    let endpoint = connection
        .route
        .request
        .proxy
        .as_ref()
        .ok_or(Error::SocksProtocol)?
        .endpoint
        .clone();
    // Record only byte counts and the actual proxy endpoint, never credentials.
    let mut rest = bytes;
    while !rest.is_empty() {
        connection.prepare_write()?;
        connection.route.gate.observe(
            &connection.route.request,
            &endpoint,
            Operation::ProxyHandshake,
        );
        let count = connection.stream.write(rest)?;
        if count == 0 {
            return Err(Error::SocksProtocol);
        }
        connection.route.gate.ledger.record(
            &connection.route.request,
            &endpoint,
            Operation::ProxyHandshake,
            count,
        );
        rest = &rest[count..];
    }
    Ok(())
}

fn receive(connection: &mut Transport, bytes: &mut [u8], deadline: Instant) -> Result<(), Error> {
    let mut rest = bytes;
    while !rest.is_empty() {
        connection.deadline = Some(deadline);
        connection.prepare_read()?;
        let count = connection.stream.read(rest)?;
        if count == 0 {
            return Err(Error::SocksProtocol);
        }
        rest = &mut rest[count..];
    }
    Ok(())
}

pub(crate) fn handshake(connection: &mut Transport, deadline: Instant) -> Result<(), Error> {
    let proxy = connection
        .route
        .request
        .proxy
        .clone()
        .ok_or(Error::SocksProtocol)?;
    let credentials = proxy.credentials.as_ref();
    send(
        connection,
        if credentials.is_some() {
            &[5, 2, 0, 2]
        } else {
            &[5, 1, 0]
        },
        deadline,
    )?;
    let mut greeting = [0; 2];
    receive(connection, &mut greeting, deadline)?;
    match (greeting, credentials) {
        ([5, 0], _) => {}
        ([5, 2], Some((user, password))) => {
            let mut auth = vec![1, user.len() as u8];
            auth.extend_from_slice(user.as_bytes());
            auth.push(password.len() as u8);
            auth.extend_from_slice(password.as_bytes());
            send(connection, &auth, deadline)?;
            let mut reply = [0; 2];
            receive(connection, &mut reply, deadline)?;
            if reply != [1, 0] {
                return Err(Error::SocksProtocol);
            }
        }
        _ => return Err(Error::SocksProtocol),
    }
    let endpoint = &connection.route.request.endpoint;
    let mut request = vec![5, 1, 0];
    match endpoint.host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            request.push(1);
            request.extend_from_slice(&ip.octets());
        }
        Ok(IpAddr::V6(ip)) => {
            request.push(4);
            request.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            request.extend_from_slice(&[3, endpoint.host.len() as u8]);
            request.extend_from_slice(endpoint.host.as_bytes());
        }
    }
    request.extend_from_slice(&endpoint.port.to_be_bytes());
    send(connection, &request, deadline)?;
    let mut reply = [0; 4];
    receive(connection, &mut reply, deadline)?;
    if reply[..3] != [5, 0, 0] {
        return Err(Error::SocksProtocol);
    }
    let length = match reply[3] {
        1 => 4,
        4 => 16,
        3 => {
            let mut length = [0];
            receive(connection, &mut length, deadline)?;
            if length[0] == 0 {
                return Err(Error::SocksProtocol);
            }
            length[0] as usize
        }
        _ => return Err(Error::SocksProtocol),
    };
    let mut bound = vec![0; length + 2];
    receive(connection, &mut bound, deadline)
}
