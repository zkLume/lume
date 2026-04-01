::  counter — stateful counter NockApp
::
::  The simplest stateful NockApp. Tracks a counter you can
::  increment, decrement, set, or reset. If you understand this
::  kernel, you understand 80% of NockApp state management.
::
::  Demonstrates:
::    - versioned-state with upgrade path
::    - poke dispatch via tagged union
::    - peek for read-only state queries
::    - effect emission
::    - soft-cast input validation
::
::  Compile: hoonc hoon/app/app.hoon $NOCK_HOME/hoon/
::
/+  lib
/=  *  /common/wrapper
::
=>
|%
::  kernel state — just a counter
::
+$  versioned-state
  $:  %v1
      count=@ud
  ==
::  effects the kernel emits
::
+$  effect  *
::  valid poke types
::
+$  cause
  $%  [%inc ~]
      [%dec ~]
      [%set n=@ud]
      [%reset ~]
  ==
--
|%
++  moat  (keep versioned-state)
::
++  inner
  |_  state=versioned-state
  ::  +load: state migration on kernel upgrade
  ::    returns old state unchanged (v1 -> v1 identity)
  ::    add version branches here when you bump to v2
  ::
  ++  load
    |=  old-state=versioned-state
    ^-  _state
    old-state
  ::  +peek: read-only state queries
  ::    /count -> current counter value
  ::
  ++  peek
    |=  =path
    ^-  (unit (unit *))
    ?+  path  ~
      [%count ~]  ``count.state
    ==
  ::  +poke: state mutations
  ::    every poke returns [effects new-state]
  ::
  ++  poke
    |=  =ovum:moat
    ^-  [(list effect) _state]
    =/  act  ((soft cause) cause.input.ovum)
    ?~  act
      ~>  %slog.[3 'counter: invalid cause']
      [~ state]
    ?-  -.u.act
      ::
        %inc
      ~>  %slog.[0 (cat 3 'count: ' (scot %ud +(count.state)))]
      :_  state(count +(count.state))
      ~[[%count +(count.state)]]
      ::
        %dec
      =/  new=@ud  ?:((gth count.state 0) (dec count.state) 0)
      ~>  %slog.[0 (cat 3 'count: ' (scot %ud new))]
      :_  state(count new)
      ~[[%count new]]
      ::
        %set
      ~>  %slog.[0 (cat 3 'count: ' (scot %ud n.u.act))]
      :_  state(count n.u.act)
      ~[[%count n.u.act]]
      ::
        %reset
      ~>  %slog.[0 'count: 0']
      :_  state(count 0)
      ~[[%count 0]]
    ==
  --
--
((moat |) inner)
