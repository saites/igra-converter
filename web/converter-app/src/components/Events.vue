<script setup lang="ts">
import Accordion from './Accordion.vue'
import Pinger from './Pinger.vue'
import { friendlyProblem, friendlyFix } from '@/utils.js'
import { computed, ref } from 'vue'

const props = defineProps({
  events: Array,
  issues: Array,
  partners: Array,
  relevant: Object,
})

const order = [
  { id: "BullRiding", name: "Bull Riding", solo: true,},
  { id: "RanchSaddleBroncRiding", name: "Ranch Saddle Bronc", solo: true,},
  { id: "SteerRiding", name: "Steer Riding", solo: true,},
  { id: "ChuteDogging", name: "Chute Dogging", solo: true,},
  { id: "CalfRopingOnFoot", name: "Calf Roping on Foot", solo: true,},
  { id: "MountedBreakaway", name: "Break-away Roping", solo: true,},

  { id: "BarrelRacing", name: "Barrel Racing", solo: true,},
  { id: "PoleBending", name: "Pole Bending", solo: true,},
  { id: "FlagRacing", name: "Flag Racing", solo: true, },

  { id: "TeamRopingHeader", name: "Team Roping Header", solo: false,},
  { id: "TeamRopingHeeler", name: "Team Roping Heeler", solo: false,},
  { id: "SteerDecorating", name: "Steer Deco", solo: false,},
  { id: "WildDragRace", name: "Wild Drag Race", solo: false,},
  { id: "GoatDressing", name: "Goat Dressing", solo: false,},
]

const isSolo = Object.fromEntries(order.map((o) => [o.id, o.solo]));

// Restructure the registration data and validation results
// into something easier to iterate over when constructing the display.
const info = computed(() => {
  let { events, relevant, partners, issues } = props
  if (!events || !relevant || !partners, !issues) { return [] }

  // Convert partner information to a map
  // of event -> [ go-round -> [ partner index -> IGRA number ] ].
  // That is, dbPartners["event"][ri][pi]
  // is the data record of the pi-th partner they listed 
  // for the ri-th go-round of event "event",
  // assuming they entered something there and it could be matched. 
  const dbPartners = partners.reduce((acc, p) => {
    const e = acc[p.event] ?? {} 
    const round_partners = e[p.round] ?? {}
    round_partners[p.index] = relevant[p.igra_number]
    e[p.round] = round_partners
    acc[p.event] = e
    return acc
  }, {})
  
  // Convert partner issues to maps.
  // problems["event"][ri][pi] gives the {problem, fix}
  // associated with the pi-th partner of the ri-th round of event "event".
  const tooFew = {}
  const tooMany = {}
  const invalidRounds = {}
  const problems = {} // unknown, unregistered, or mismatched
  issues.filter((i) => i?.problem?.data?.event && i?.fix).forEach((i) => {
    const pdata = i.problem.data

    if (i.problem.name === "TooFewPartners") {
      const tf_e = tooFew[pdata.event] ?? {}
      tf_e[pdata.round] = true
      tooFew[pdata.event] = tf_e
      return
    } else if (i.problem.name === "TooManyPartners") {
      const tm_e = tooMany[pdata.event] ?? {}
      tm_e[pdata.round] = true
      tooMany[pdata.event] = tm_e
      return
    } else if (i.problem.name === "InvalidRoundID") {
      const ir_e = invalidRounds[pdata.event] ?? {}
      ir_e[pdata.round] = true
      invalidRounds[pdata.event] = ir_e
      return
    }
    
    if (pdata.index === undefined) {
      return
    }
    
    const e = problems[pdata.event] ?? {}
    const r = e[pdata.round] ?? {}
    const prob_to_fixes = r[pdata.index] ?? {}
    const fixes = prob_to_fixes[i.problem.name] ?? []
    fixes.push(i.fix)
    prob_to_fixes[i.problem.name] = fixes
    r[pdata.index] = prob_to_fixes
    e[pdata.round] = r
    problems[pdata.event] = e
  }, {})

  // info gathers together the partner info for the event,
  // constructing an object of the above values.
  const info = events.reduce((acc, e) => {
    const r = acc[e.eventId] ?? {
      "rounds": {1: false, 2: false}, 
      "partners": dbPartners[e.eventId] ?? {},
      "regPartners": {},
      "problems": problems[e.eventId] ?? {},
      "tooFew": tooFew[e.eventId] ?? {},
      "tooMany": tooMany[e.eventId] ?? {},
      "invalidRounds": invalidRounds[e.eventId] ?? {},
    }

    r.rounds[e.round] = true 
    r.regPartners[e.round] = e.partners
    acc[e.eventId] = r
    return acc
  }, {})

  // Present the registration information in a consistent order,
  // defined by the above collection.
  return order.map((o) => { 
    return {
      "o": o, 
      "name": o.name, 
      // info["some event"].rounds[i] is True 
      // iff this person is registered for that event and go-round.
      "rounds": info[o.id]?.rounds ?? {1: false, 2: false},
      // regPartners[round][i] is the i-th partner they listed when registering.
      "regPartners": info[o.id]?.regPartners ?? {}, 
      // partners[round][i] is the IGRA number
      // of the i-th partner they listed when registering, if found.
      "partners": info[o.id]?.partners ?? {}, 
      "problems": info[o.id]?.problems ?? {}, 
      "tooFew": info[o.id]?.tooFew ?? {},
      "tooMany": info[o.id]?.tooMany ?? {},
      "invalidRounds": info[o.id]?.invalidRounds ?? {},
    }
  })
})

const soloEvents = computed(() => {
  return info?.value?.filter((r) => r.o.solo) ?? [];
})

const partnerEvents = computed(() => {
  return info?.value?.filter((r) => !r.o.solo) ?? [];
})

const showEvents = ref(false);
</script>

<template>
  <accordion> 
    <template #summary>
      <header class="ps-4 w-full text-lg bg-green-300">
      {{events.length}} Go-Rounds
      </header>
    </template>

    <template #content>
      <div class="item-grid my-2">
        <div class="col-span-3 grid grid-cols-3 bg-slate-300 w-full">
          <th>Event</th>
          <th>1st Go</th>
          <th>2nd Go</th>
        </div>

        <template v-for="e in info">
          <div class="item text-end font-bold">{{e.name}}</div>
          <div class="item text-center">{{e.rounds[1] ? "X" : ""}}</div>
          <div class="item text-center">{{e.rounds[2] ? "X" : ""}}</div>

          <template v-if="e.o.solo">
            <template v-for="(_, round_i) in e.rounds">
                <div v-if="e.invalidRounds[round_i]" class="err place-self-start mx-4 col-span-3 w-full">
                  <pinger color="bg-red-800">
                    <span class="mx-4 ps-4">This event wasn't expected to have go-round {{round_i}}.
                      This is likely a developer bug :-/
                    </span>
                  </pinger>
                </div>
                
                <div v-if="e.tooMany[round_i]" class="err ms-4 my-2 col-span-3 w-full">
                  <pinger color="bg-red-800">
                    <span class="ps-4">There are too many partners listed for this go-round.
                      This is likely a developer bug :-/
                    </span>
                  </pinger>
                </div>
            </template>
          </template>

          <template v-else>
            <template v-for="(reg_part, round_i) in e.regPartners">
              <div class="place-self-start px-4 pb-2 col-span-3 w-full">

                <header>Go {{round_i}} Partners:</header>
                
                <div v-if="e.invalidRounds[round_i]" class="err ms-4 my-2">
                  <pinger color="bg-red-800">
                    <span class="ps-4">This event wasn't expected to have go-round {{round_i}}.
                      This is likely a developer bug :-/
                    </span>
                  </pinger>
                </div>
                
                <div v-if="e.tooMany[round_i]" class="err ms-4 my-2">
                  <pinger color="bg-red-800">
                    <span class="ps-4">There are too many partners listed for this go-round.
                      This is likely a developer bug :-/
                    </span>
                  </pinger>
                </div>

                <div v-if="e.tooFew[round_i]" class="err ms-4 my-2" >
                  <pinger color="bg-red-800">
                    <span class="ps-4">The registrant did not list enough partners for this go-round.</span>
                  </pinger>
                </div>

                <div class="ps-4" v-for="(p, reg_pi) in reg_part">
                  Partner {{reg_pi + 1}}: {{p ?? "----"}}

                  <div v-if="e.partners[round_i]?.[reg_pi]"
                      class="ps-4" 
                      :class="{'good': !e.problems[round_i]?.[reg_pi],
                               'err': e.problems[round_i]?.[reg_pi], }"
                    >
                    Found Match: 
                    {{e.partners[round_i][reg_pi].igra_number}} 
                    {{e.partners[round_i][reg_pi].first_name}} 
                    {{e.partners[round_i][reg_pi].last_name}} 

                    <div class="err" 
                      v-if="e.problems[round_i]?.[reg_pi]?.UnregisteredPartner">
                      This partner is not registered for this rodeo.
                    </div>
                    
                    <div class="err" 
                      v-if="e.problems[round_i]?.[reg_pi]?.MismatchedPartners">
                      This partner is registered for this rodeo,
                      but they do not mutually list the registrant as partner for this go-round.
                    </div>

                  </div>

                  <div v-else-if="!e.tooFew[round_i]" class="err ps-4">
                    No perfect match. 
                    <ul v-if="e.problems[round_i]?.[reg_pi]?.UnknownPartner?.map((fix) => relevant[fix.data]).filter((pm) => pm)">
                      <li v-for="pm in e.problems[round_i]?.[reg_pi]?.UnknownPartner?.map((fix) => relevant[fix.data]).filter((pm) => pm)">
                        Possible Match: 
                        {{pm.igra_number}} |
                        {{pm.legal_first}}
                        {{pm.legal_last}}
                        <template v-if="pm.first_name !== pm.legal_first || pm.last_name !== pm.legal_last">
                        aka {{pm.first_name}} {{pm.last_name}}
                        </template>
                      </li>
                    </ul>
                  </div>
                </div>

              </div>
            </template>
          </template>
        </template>
      </div>
    </template>
  </accordion>
</template>

<style>
.item-grid {
  @apply grid grid-cols-3 justify-items-center pb-2 bg-gray-100;
  @apply border-y-2 border-y-indigo-500 border-x-2 border-x-indigo-300 w-full;
}

.item {
  @apply border-t-2 border-indigo-500 w-full py-1;
}

.err {
  @apply bg-red-300;
}
.good {
  @apply bg-green-300;
}
</style>

