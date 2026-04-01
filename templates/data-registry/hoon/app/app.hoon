::  data-registry — commitment-based data registry NockApp
::
::  Register named data commitments. Each registration stores the
::  SHA-256 hash of the data. Verify that data matches a registered
::  commitment. Look up registered hashes by name.
::
::  This is the generalized pattern behind any NockApp that needs
::  to prove "I committed to this data before you asked." Document
::  hashes, configuration digests, model weights, whatever — if
::  you can hash it, you can register it.
::
::  Demonstrates:
::    - map-based state (name -> hash registry)
::    - cryptographic commitment (shax / SHA-256)
::    - data verification against commitments
::    - entry counting
::
::  Compile: hoonc hoon/app/app.hoon $NOCK_HOME/hoon/
::
/+  lib
/=  *  /common/wrapper
::
=>
|%
::  kernel state — name-to-hash registry
::
+$  versioned-state
  $:  %v1
      registry=(map @t @)
      entries=@ud
  ==
::
+$  effect  *
::
+$  cause
  $%  [%register name=@t dat=@]
      [%verify name=@t dat=@]
      [%lookup name=@t]
  ==
--
|%
++  moat  (keep versioned-state)
::
++  inner
  |_  state=versioned-state
  ::
  ++  load
    |=  old-state=versioned-state
    ^-  _state
    old-state
  ::  +peek: query the registry
  ::    /entries     -> number of registered entries
  ::    /hash/<name> -> hash for a specific name
  ::
  ++  peek
    |=  =path
    ^-  (unit (unit *))
    ?+  path  ~
      [%entries ~]
        ``entries.state
      [%hash name=@t ~]
        =/  key  +<.path
        ``(~(get by registry.state) key)
    ==
  ::  +poke: register, verify, or look up data
  ::
  ++  poke
    |=  =ovum:moat
    ^-  [(list effect) _state]
    =/  act  ((soft cause) cause.input.ovum)
    ?~  act
      ~>  %slog.[3 'registry: invalid cause']
      [~ state]
    ?-  -.u.act
      ::  %register — hash the data and store under name
      ::
        %register
      =/  hash=@  (shax dat.u.act)
      =/  new-reg  (~(put by registry.state) name.u.act hash)
      ~>  %slog.[0 (cat 3 'registered: ' name.u.act)]
      :_  state(registry new-reg, entries +(entries.state))
      ~[[%registered name.u.act hash]]
      ::  %verify — check if data matches the registered hash
      ::
        %verify
      =/  existing  (~(get by registry.state) name.u.act)
      ?~  existing
        ~>  %slog.[3 (cat 3 'not found: ' name.u.act)]
        :_  state
        ~[[%not-found name.u.act]]
      =/  hash=@  (shax dat.u.act)
      =/  valid=?  =(hash u.existing)
      ~>  %slog.[0 (cat 3 ?:(valid 'verified: ' 'mismatch: ') name.u.act)]
      :_  state
      ~[[%verified name.u.act valid]]
      ::  %lookup — return the registered hash (if any)
      ::
        %lookup
      =/  existing  (~(get by registry.state) name.u.act)
      ?~  existing
        :_  state
        ~[[%not-found name.u.act]]
      :_  state
      ~[[%found name.u.act u.existing]]
    ==
  --
--
((moat |) inner)
