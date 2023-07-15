<script setup lang="ts">
import { friendlyProblem, friendlyFix } from '@/utils.js'
import { computed } from 'vue'

const props = defineProps({
  event: Object,
  issues: Array,
  partners: Array,
  relevant: Object,
})


const theseIssues = computed(() => {
  let { issues, event } = props;
  if (!issues || !event) { return []; }

  return issues.filter((issue) => {
      return (
        issue.problem.data && issue.problem.data.event === event.rodeoEventRelId
          && (
            !issue.problem.data.round
            || issue.problem.data.round === event.round
          )
      )
  })
})

const bgColor = computed(() => {
  return theseIssues.value.length ? "bg-red-400" : "#f9f9f9"
})

function problemName(issue) {
  return friendlyProblem(issue)
}

function suggestedFix(issue) {
  if (issue.fix.name === "UseThisRecord") {
    let other = props.relevant[issue.fix.data];
    if (other) {
      return `Maybe they meant ${other.first_name} ${other.last_name} with ID ${other.igra_number}?`
    }
  }
  return friendlyFix(issue)
}

const partnerInfo = computed(() => {
  let { event, partners, relevant } = props
  if (!event.partners || !partners) {
    return []
  }

  let eventPartners = partners.filter((p) => {
    return p.event === event.rodeoEventRelId && p.round === event.round
  });

  // TODO: this logic is buggy and misassociated some people.
  // Need to either move the logic over to the server,
  // or figure out what's broken here.
  // match the user value to the correct partner.
  return event.partners.map((nameOrNum) => {
    let lowerNN = nameOrNum.trim().toLowerCase()
    if (lowerNN === "") { return {} }

    // among their known partners, find a match
    // if the lowerNN contains the IGRA number or partner name
    return eventPartners.find((p) => {
      if (lowerNN.includes(p.igra_number)) { return true }

      let dbPartner = relevant[p.igra_number]
      if (!dbPartner) { return false }

      let partnerName = `${dbPartner.first_name} ${dbPartner.last_name}`.toLowerCase()
      let partnerLegalName = `${dbPartner.legal_first} ${dbPartner.legal_last}`.toLowerCase()
      return lowerNN.includes(partnerName) || lowerNN.includes(partnerLegalName)
    })
  }).map((p) => { return p ? relevant[p.igra_number] : undefined })
})

function formatPartner(p) {
  if (!p) { return }
  return `${p.igra_number} ${p.first_name} ${p.last_name}`
}

const isSolo = {
  "FlagRacing": true,
  "ChuteDogging": true,
  "CalfRopingOnFoot": true,
  "SteerRiding": true,
  "RanchSaddleBroncRiding": true,
  "BullRiding": true,
  "PoleBending": true,
  // Team events
  "TeamRopingHeader": false,
  "TeamRopingHeller": false,
  "WildDragRace": false,
  "GoatDressing": false,
  "SteerDeco": false,
}

function collectEvents(events, findSolo) {
  return events.filter((e) => findSolo ^ !isSolo[e.rodeoEventRelId])
    .reduce((acc, e) => {
      const entry = acc[e.rodeoEventRelId] ?? {
        "name": e.rodeoEventRelId,
        "rounds": [],
      }
      entry.rounds.push(e.round)
      acc[e.rodeoEventRelId] = entry
      return acc
    }, {})
}

const partnerEvents = computed(() => {
  let { events } = props
  if (!events) { return [] }
  return collectEvents(events, false)
})

const soloEvents = computed(() => {
  let { events } = props
  if (!events) { return {} }
  return collectEvents(events, true)
})

</script>

<template>
  <div class="grid grid-cols-4">
      
    <th>Event</th>
    <th>Round</th>
    <th>Partner 1</th>
    <th>Partner 2</th>
    
    <header v-if="soloEvents.length > 0"
      class="text-lg grid-col-span-4">Solo Events</header>

    <div class="grid grid-cols-3" v-for="e in soloEvents">
      <div>{{e.name}}</div>
      <div>{{e.rounds[0] ? "X" : ""}}</div>
      <div>{{e.rounds[1] ? "X" : ""}}</div>
    </div>
      
      <header class="text-lg">Partner Events</header>
      <div class="grid grid-cols-3" v-for="e in partnerEvents">
        <div>{{e.name}}</div>
        <div>{{e.rounds[0] ? "X" : ""}}</div>
        <div>{{e.rounds[1] ? "X" : ""}}</div>
      </div>
  </div>

      <table class="table-auto">
        <thead>
          <tr>
          </tr>
        </thead>
        <tbody>
        </tbody>
      </table>

  <tr :class="bgColor" class="text-center">
    <td>{{event.rodeoEventRelId}}</td>
    <td>{{event.round}}</td>
    <td>{{event.partners[0]}}</td>
    <td>{{event.partners[1]}}</td>
  </tr>
  <tr :class="bgColor" class="text-left text-sm" v-for="issue in theseIssues">
    <td colspan="4">
      → <span>{{problemName(issue)}}</span> <span>{{suggestedFix(issue)}}</span>
    </td>
  </tr>
  <tr>
    <td></td>
    <td></td>
    <td v-if="partnerInfo[0]">Match: {{formatPartner(partnerInfo[0])}}</td>
    <td v-else></td>
    <td v-if="partnerInfo[1]">Match: {{formatPartner(partnerInfo[1])}}</td>
    <td v-else></td>
  </tr>
  <tr>
    <td></td>
    <td></td>
    <td></td>
    <td></td>
  </tr>
</template>

